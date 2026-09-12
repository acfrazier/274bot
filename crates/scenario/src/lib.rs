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
    cheat, close_modal, op_loc, tele_args, Driver, Interactions, SendReason, SendResult,
    MAXME_SETSTATS,
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
        "alcher_defaults" => Some(alcher_defaults_scenario()),
        "alcher_custom" => Some(alcher_custom_scenario()),
        "alcher_custom_alias" => Some(alcher_custom_alias_scenario()),
        "alcher_custom_name" => Some(alcher_custom_name_scenario()),
        "alcher_ordered" => Some(alcher_ordered_scenario()),
        "alcher_large_batch" => Some(alcher_large_batch_scenario()),
        "bank_fletcher" => Some(bank_fletcher_scenario()),
        "bank_fletcher_shafts" => Some(bank_fletcher_shafts_scenario()),
        "bank_fletcher_headless" => Some(bank_fletcher_headless_scenario()),
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
        "wildy_agility" => Some(wildy_agility_scenario()),
        "brimhaven_agility" => Some(brimhaven_agility_scenario()),
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
        "rune_crafter" => Some(rune_crafter_scenario()),
        "rune_crafter_earth" => Some(rune_crafter_earth_scenario()),
        "mule_crafter" => Some(mule_crafter_scenario()),
        "ardy_cakes" => Some(ardy_cakes_scenario()),
        "ardy_cakes_fight" => Some(ardy_cakes_fight_scenario()),
        "ardy_thiever" => Some(ardy_thiever_scenario()),
        "ardy_thiever_fight" => Some(ardy_thiever_fight_scenario()),
        "ardy_thiever_knight" => Some(ardy_thiever_knight_scenario()),
        "gnome_chop" => Some(gnome_chop_scenario()),
        "gnome_fletch_short" => Some(gnome_fletch_short_scenario()),
        "gnome_fletch_long" => Some(gnome_fletch_long_scenario()),
        "coal_trucks" => Some(coal_trucks_scenario()),
        "cook_bot" => Some(cook_bot_scenario()),
        "cook_bot_lobster" => Some(cook_bot_lobster_scenario()),
        "smelter_bot" => Some(smelter_bot_scenario()),
        "smelter_bot_steel" => Some(smelter_bot_steel_scenario()),
        "flax_spinner" => Some(flax_spinner_scenario()),
        "flax_aio" => Some(flax_aio_scenario()),
        "flax_aio_pick" => Some(flax_aio_pick_scenario()),
        "flax_aio_spin" => Some(flax_aio_spin_scenario()),
        "herblore_secondaries" => Some(herblore_secondaries_scenario()),
        "herblore_secondaries_newt" => Some(herblore_secondaries_newt_scenario()),
        "chaos_druid" => Some(chaos_druid_scenario()),
        "chaos_druid_tower" => Some(chaos_druid_tower_scenario()),
        "chaos_druid_yanille" => Some(chaos_druid_yanille_scenario()),
        "moss_giant" => Some(moss_giant_scenario()),
        "hill_giant" => Some(hill_giant_scenario()),
        "auto_fighter" => Some(auto_fighter_scenario()),
        "auto_fighter_mage" => Some(auto_fighter_mage_scenario()),
        "auto_fighter_range" => Some(auto_fighter_range_scenario()),
        "rock_crab" => Some(rock_crab_scenario()),
        "rock_crab_range" => Some(rock_crab_range_scenario()),
        "green_dragon" => Some(green_dragon_scenario()),
        "green_dragon_special" => Some(green_dragon_special_scenario()),
        "green_dragon_potions" => Some(green_dragon_potions_scenario()),
        "fire_giant" => Some(fire_giant_scenario()),
        "ardy_fighter" => Some(ardy_fighter_scenario()),
        "auto_fighter_bank" => Some(auto_fighter_bank_scenario()),
        "moss_giant_bank" => Some(moss_giant_bank_scenario()),
        "hill_giant_bank" => Some(hill_giant_bank_scenario()),
        "chaos_druid_bank" => Some(chaos_druid_bank_scenario()),
        "ardy_fighter_bank" => Some(ardy_fighter_bank_scenario()),
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
        "alcher_defaults",
        "alcher_custom",
        "alcher_custom_alias",
        "alcher_custom_name",
        "alcher_ordered",
        "alcher_large_batch",
        "bank_fletcher",
        "bank_fletcher_shafts",
        "bank_fletcher_headless",
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
        "wildy_agility",
        "brimhaven_agility",
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
        "rune_crafter",
        "rune_crafter_earth",
        "mule_crafter",
        "ardy_cakes",
        "ardy_cakes_fight",
        "ardy_thiever",
        "ardy_thiever_fight",
        "ardy_thiever_knight",
        "gnome_chop",
        "gnome_fletch_short",
        "gnome_fletch_long",
        "coal_trucks",
        "cook_bot",
        "cook_bot_lobster",
        "smelter_bot",
        "smelter_bot_steel",
        "flax_spinner",
        "flax_aio",
        "flax_aio_pick",
        "flax_aio_spin",
        "herblore_secondaries",
        "herblore_secondaries_newt",
        "chaos_druid",
        "chaos_druid_tower",
        "chaos_druid_yanille",
        "moss_giant",
        "hill_giant",
        "auto_fighter",
        "auto_fighter_mage",
        "auto_fighter_range",
        "rock_crab",
        "rock_crab_range",
        "green_dragon",
        "green_dragon_special",
        "green_dragon_potions",
        "fire_giant",
        "ardy_fighter",
        "auto_fighter_bank",
        "moss_giant_bank",
        "hill_giant_bank",
        "chaos_druid_bank",
        "ardy_fighter_bank",
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

const ALCHER_DEFAULTS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&[]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

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
const YEW_LONGBOW_ID: i32 = 855;
const CERT_YEW_LONGBOW_ID: i32 = 856;
const NATURE_RUNE_ID: i32 = 561;
const COINS_ID: i32 = 995;
const STAFF_OF_FIRE_ID: i32 = 1387;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
/// High Level Alchemy pays 60% of shop cost: floor(1280 * 0.6) = 768.
const YEW_LONGBOW_ALCH_COINS: i32 = 768;
const HIGH_ALCH_MAGIC_XP: i32 = 65;

fn alcher_custom_alias_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_alias", ALCHER_CUSTOM_ALIAS_INJECT)
}

fn alcher_custom_name_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_name", ALCHER_CUSTOM_NAME_INJECT)
}

/// Empty `items` selects the frozen catalog's DEFAULT_ALCH_ITEMS. Only one
/// default target is banked, so the noted withdrawal proves fallback selection.
fn alcher_defaults_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: 6,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed only one default Alcher target at Varrock West before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 55");
                cheat(c, "givebank yew_longbow 1");
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
            "confirm no seeded unnoted Yew longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted Yew longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
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
        (
            "confirm no Rune chainbody fallback confounder before Start",
            Proof::ItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the sole unnoted default target seed",
        Proof::BankItemId {
            id: YEW_LONGBOW_ID,
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
            "acknowledge no seeded noted Yew longbow in bank",
            Proof::BankItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no Rune chainbody in bank",
            Proof::BankItemAtMost {
                name: "Rune chainbody",
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
            "watch default fallback withdraw the noted Yew longbow id",
            Proof::ItemId {
                id: CERT_YEW_LONGBOW_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from the default High Alchemy cast", xp),
        (
            "watch the noted default target consumed",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
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
            "watch the exact default High Alchemy coin increase",
            Proof::ItemId {
                id: COINS_ID,
                count: YEW_LONGBOW_ALCH_COINS,
            },
        ),
        (
            "confirm Rune chainbody never entered the fallback path",
            Proof::ItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_defaults",
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
            script_settings_inject: Some(ALCHER_DEFAULTS_INJECT),
            terminal_shot: Some("alcher_defaults"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
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

const BANK_FLETCHER_SHAFTS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Arrow shafts"),
    },
];

const BANK_FLETCHER_HEADLESS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Headless arrows"),
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

const LOGS_ID: i32 = 1511;
const WILLOW_LOGS_ID: i32 = 1519;
const ARROW_SHAFT_ID: i32 = 52;
const HEADLESS_ARROW_ID: i32 = 53;
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

#[derive(Clone, Copy)]
struct BankFletcherOption {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    primary_alias: &'static str,
    primary_id: i32,
    primary_carried: i32,
    primary_banked: i32,
    primary_restocked: i32,
    secondary: Option<(&'static str, i32, i32, i32, i32)>,
    product_id: i32,
    first_product_count: i32,
    keep_knife: bool,
}

fn bank_fletcher_option_scenario(option: BankFletcherOption) -> Scenario {
    let BankFletcherOption {
        name,
        inject,
        primary_alias,
        primary_id,
        primary_carried,
        primary_banked,
        primary_restocked,
        secondary,
        product_id,
        first_product_count,
        keep_knife,
    } = option;
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed exact BankFletcher option inputs and bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat fletching 1");
                if keep_knife {
                    cheat(c, "give knife 1");
                }
                cheat(c, &format!("give {primary_alias} {primary_carried}"));
                cheat(c, &format!("givebank {primary_alias} {primary_banked}"));
                if let Some((alias, _, carried, banked, _)) = secondary {
                    cheat(c, &format!("give {alias} {carried}"));
                    cheat(c, &format!("givebank {alias} {banked}"));
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
    steps.push(bank_fletcher_watch(
        "confirm Fletching 1 before Start",
        Proof::Stat { id: 9, min: 1 },
    ));
    if keep_knife {
        steps.push(bank_fletcher_watch(
            "confirm the knife before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: 1,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "confirm the exact primary input before Start",
        Proof::ItemId {
            id: primary_id,
            count: primary_carried,
        },
    ));
    if let Some((_, id, carried, _, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "confirm the exact secondary input before Start",
            Proof::ItemId { id, count: carried },
        ));
    }
    steps.push(bank_fletcher_watch(
        "confirm no seeded product in pack before Start",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact primary seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: primary_banked,
        },
    ));
    if let Some((_, id, _, banked, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact secondary seed bank",
            Proof::BankItemId { id, count: banked },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded product in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Fletching XP from the first exact input batch",
            Proof::StatXpGain { id: 9, min: 1 },
        ),
        (
            "watch the exact first product batch",
            Proof::ItemId {
                id: product_id,
                count: first_product_count,
            },
        ),
        (
            "watch the primary input consumed",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if let Some((_, id, _, _, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "watch the secondary input consumed",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(bank_fletcher_watch(
        "watch the exact product batch enter a fresh bank",
        Proof::BankItemId {
            id: product_id,
            count: first_product_count,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch the product leave the pack after deposit",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch fresh primary input restocked",
        Proof::ItemId {
            id: primary_id,
            count: primary_restocked,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch primary bank stock decrease on restock",
        Proof::BankItemIdAtMost {
            id: primary_id,
            count: primary_banked - primary_restocked,
        },
    ));
    if let Some((_, id, _, banked, restocked)) = secondary {
        steps.push(bank_fletcher_watch(
            "watch fresh secondary input restocked",
            Proof::ItemId {
                id,
                count: restocked,
            },
        ));
        steps.push(bank_fletcher_watch(
            "watch secondary bank stock decrease on restock",
            Proof::BankItemIdAtMost {
                id,
                count: banked - restocked,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "watch the script close its production bank",
        Proof::BankClosed,
    ));
    steps.push(bank_fletcher_watch(
        "watch fresh Fletching XP after the restock and closed return",
        Proof::FreshStatXpGain { id: 9, min: 1 },
    ));
    steps.push(bank_fletcher_watch(
        "watch further exact product after restock",
        Proof::ItemId {
            id: product_id,
            count: 1,
        },
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: product_id,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn bank_fletcher_shafts_scenario() -> Scenario {
    bank_fletcher_option_scenario(BankFletcherOption {
        name: "bank_fletcher_shafts",
        inject: BANK_FLETCHER_SHAFTS_INJECT,
        primary_alias: "logs",
        primary_id: LOGS_ID,
        primary_carried: 27,
        primary_banked: 54,
        primary_restocked: 27,
        secondary: None,
        product_id: ARROW_SHAFT_ID,
        first_product_count: 405,
        keep_knife: true,
    })
}

fn bank_fletcher_headless_scenario() -> Scenario {
    bank_fletcher_option_scenario(BankFletcherOption {
        name: "bank_fletcher_headless",
        inject: BANK_FLETCHER_HEADLESS_INJECT,
        primary_alias: "feather",
        primary_id: FEATHER_ID,
        primary_carried: 30,
        primary_banked: 60,
        primary_restocked: 60,
        secondary: Some(("arrow_shaft", ARROW_SHAFT_ID, 30, 60, 60)),
        product_id: HEADLESS_ARROW_ID,
        first_product_count: 30,
        keep_knife: false,
    })
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

const WILDY_START: WorldTile = WorldTile {
    x: 2998,
    z: 3916,
    level: 0,
};
/// North of the selected inner Gate at (2998,3931). Radius 2 cannot include
/// the gate tile, so a ridge click without the world crossing fails closed.
const WILDY_AFTER_RIDGE: WorldTile = WorldTile {
    x: 2998,
    z: 3934,
    level: 0,
};
/// Selected m46_61 loc 2288 at (3004,3938); rs2 lands at loc z+9.
const WILDY_PIPE_DEST: WorldTile = WorldTile {
    x: 3004,
    z: 3947,
    level: 0,
};
/// Selected m46_61 loc 2283 at (3005,3952); rs2 lands five north of its stand.
const WILDY_ROPE_DEST: WorldTile = WorldTile {
    x: 3005,
    z: 3958,
    level: 0,
};
/// Selected m46_61 loc 2311 at (3001,3960); the sixth jump lands x-5.
const WILDY_STONE_DEST: WorldTile = WorldTile {
    x: 2996,
    z: 3960,
    level: 0,
};
/// Selected m46_61 loc 2297 is raw plane 1 over a LinkBelow bridge; the
/// player remains on observed scene plane 0 while the moves land x-7.
const WILDY_LOG_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3945,
    level: 0,
};
/// Centre tile of the selected three-wide rocks 2328; rs2 lands three south.
const WILDY_ROCKS_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3933,
    level: 0,
};

const BRIMHAVEN_ENTRANCE: WorldTile = WorldTile {
    x: 2809,
    z: 3194,
    level: 0,
};
/// Selected ladder 3617 Climb-Down destination, also arena platform 24.
const BRIMHAVEN_LADDER_LANDING: WorldTile = WorldTile {
    x: 2805,
    z: 9590,
    level: 3,
};
const AGILITY_TICKET_ID: i32 = 2996;
const AGILITY_ARENA_VARP: i32 = 309;

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

const WILDY_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "acquireFoodAtStart",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "minFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const BRIMHAVEN_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stealRestock",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtTickets",
        value: ScriptInjectValue::Num(1000.0),
    },
];

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

/// Cross the south ridge, complete the five selected wilderness obstacles and
/// make real progress through the next pipe. All setup cheats happen before
/// catalog Start; post-Start steps are observation-only.
fn wildy_agility_scenario() -> Scenario {
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 598,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52 and five Lobsters, then tele south of the ridge",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "setstat hitpoints 40");
                cheat(c, "give lobster 5");
                cheat(
                    c,
                    &tele_args(WILDY_START.level, WILDY_START.x, WILDY_START.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: WILDY_START.x,
                z: WILDY_START.z,
                level: WILDY_START.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm five exact Lobsters before Start",
        Proof::ItemId {
            id: LOBSTER_ID,
            count: 5,
        },
    ));
    steps.push(drain_advancestat());
    steps.push(bank_fletcher_watch(
        "confirm Hitpoints 40 before the wilderness course",
        Proof::Stat {
            id: HITPOINTS_STAT,
            min: 40,
        },
    ));
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch ridge Agility XP",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 15,
            },
        ),
        (
            "watch the ridge world crossing north of the inner gate",
            Proof::ArrivedNear {
                x: WILDY_AFTER_RIDGE.x,
                z: WILDY_AFTER_RIDGE.z,
                level: WILDY_AFTER_RIDGE.level,
                radius: 2,
            },
        ),
        (
            "watch pipe XP after the ridge",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 27,
            },
        ),
        (
            "watch the selected pipe destination",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch ropeswing XP after the pipe",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 47,
            },
        ),
        (
            "watch the selected ropeswing destination",
            Proof::ArrivedNear {
                x: WILDY_ROPE_DEST.x,
                z: WILDY_ROPE_DEST.z,
                level: WILDY_ROPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch stepping-stone XP after the ropeswing",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 67,
            },
        ),
        (
            "watch the selected stepping-stone destination",
            Proof::ArrivedNear {
                x: WILDY_STONE_DEST.x,
                z: WILDY_STONE_DEST.z,
                level: WILDY_STONE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch log XP after the stepping stones",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 87,
            },
        ),
        (
            "watch the selected log destination",
            Proof::ArrivedNear {
                x: WILDY_LOG_DEST.x,
                z: WILDY_LOG_DEST.z,
                level: WILDY_LOG_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the five-obstacle lap XP bonus",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 586,
            },
        ),
        (
            "watch the selected rocks destination",
            Proof::ArrivedNear {
                x: WILDY_ROCKS_DEST.x,
                z: WILDY_ROCKS_DEST.z,
                level: WILDY_ROCKS_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the next pipe destination after the full lap",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "wildy_agility",
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
            start_script: Some("WildyAgility"),
            script_settings_inject: Some(WILDY_AGILITY_INJECT),
            terminal_shot: Some("wildy_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Pay and enter naturally, require obstacle XP before the script's first
/// no-ticket Tag, earn a later ticket, then require independently fresh obstacle
/// XP after that ticket.
fn brimhaven_agility_scenario() -> Scenario {
    let first_hop_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let subsequent_xp = Proof::FreshStatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52, 1000 Coins and ten Lobsters, then tele to the arena entrance",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "give coins 1000");
                cheat(c, "give lobster 10");
                cheat(
                    c,
                    &tele_args(
                        BRIMHAVEN_ENTRANCE.level,
                        BRIMHAVEN_ENTRANCE.x,
                        BRIMHAVEN_ENTRANCE.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_ENTRANCE.x,
                z: BRIMHAVEN_ENTRANCE.z,
                level: BRIMHAVEN_ENTRANCE.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm 1000 exact Coins before Start",
            Proof::ItemId {
                id: COINS_ID,
                count: 1000,
            },
        ),
        (
            "confirm ten exact Lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 10,
            },
        ),
        (
            "confirm no seeded agility-arena ticket before Start",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the 200-Coin arena fee",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 800,
            },
        ),
        (
            "watch the arena paid bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 2,
            },
        ),
        (
            "watch Climb-Down reach the arena ladder platform",
            Proof::ArrivedNear {
                x: BRIMHAVEN_LADDER_LANDING.x,
                z: BRIMHAVEN_LADDER_LANDING.z,
                level: BRIMHAVEN_LADDER_LANDING.level,
                radius: 2,
            },
        ),
        (
            "watch obstacle XP from a real hop before the first Tag",
            first_hop_xp,
        ),
        (
            "watch the first Tag prompt for the next pillar",
            Proof::Chat {
                needle: "tag the next",
            },
        ),
        (
            "watch the first Tag set the tagged bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 15,
            },
        ),
        (
            "confirm the first Tag grants no ticket",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
        (
            "watch a later Tag grant the first ticket",
            Proof::ItemId {
                id: AGILITY_TICKET_ID,
                count: 1,
            },
        ),
        (
            "watch subsequent arena obstacle XP after the ticket",
            subsequent_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "brimhaven_agility",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: subsequent_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BrimhavenAgility"),
            script_settings_inject: Some(BRIMHAVEN_AGILITY_INJECT),
            terminal_shot: Some("brimhaven_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
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

const RUNECRAFT_STAT: i32 = 20;
const RUNE_ESSENCE_ID: i32 = 1436;
const NOTED_ESSENCE_ID: i32 = 1437;
const AIR_TALISMAN_ID: i32 = 1438;
const EARTH_TALISMAN_ID: i32 = 1440;
const AIR_RUNE_ID: i32 = 556;
const EARTH_RUNE_ID: i32 = 557;
const RUNE_ESSENCE_SEED: i32 = 200;

const RUNECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2988,
    z: 3294,
    level: 0,
};
const RUNECRAFTER_EARTH_RUINS: WorldTile = WorldTile {
    x: 3303,
    z: 3477,
    level: 0,
};
const MULECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2983,
    z: 3288,
    level: 0,
};
const VARROCK_EAST_BANK: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};

const RUNE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const RUNE_CRAFTER_EARTH_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Earth runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const MULE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air rune"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Crafter"),
    },
    ScriptSettingInject {
        id: "partner",
        value: ScriptInjectValue::Str(""),
    },
    ScriptSettingInject {
        id: "bankFill",
        value: ScriptInjectValue::Bool(true),
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

fn rune_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: RUNECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

fn rune_crafter_earth_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter_earth",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_EARTH_INJECT,
        bank: VARROCK_EAST_BANK,
        ruins: RUNECRAFTER_EARTH_RUINS,
        rc_level: 9,
        talisman_alias: "earth_talisman",
        talisman_id: EARTH_TALISMAN_ID,
        rune_id: EARTH_RUNE_ID,
        wrong_rune_id: AIR_RUNE_ID,
    })
}

fn mule_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "mule_crafter",
        start_script: "MuleCrafter",
        inject: MULE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: MULECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

fn runecraft_open_seed_bank(name: &'static str, arm: Proof) -> Step {
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

/// Empty pack at the selected bank. Banked unnoted essence 1436 and the
/// selected talisman; never crafted runes or noted 1437. Script withdraws,
/// uses the talisman on the selected Mysterious ruins, Craft-rune, banks the
/// produced runes, restocks essence and crafts again. Trade/paired modes stay
/// pending.
struct RuneCraftSpec {
    name: &'static str,
    start_script: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    ruins: WorldTile,
    rc_level: i32,
    talisman_alias: &'static str,
    talisman_id: i32,
    rune_id: i32,
    wrong_rune_id: i32,
}

fn rune_craft_variant(spec: RuneCraftSpec) -> Scenario {
    let RuneCraftSpec {
        name,
        start_script,
        inject,
        bank,
        ruins,
        rc_level,
        talisman_alias,
        talisman_id,
        rune_id,
        wrong_rune_id,
    } = spec;
    let ruins_near = Proof::ArrivedNear {
        x: ruins.x,
        z: ruins.z,
        level: ruins.level,
        radius: 4,
    };
    let crafted = Proof::ItemId {
        id: rune_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed runecraft, banked essence, talisman, and tele before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat runecraft {rc_level}"));
                cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                cheat(c, &format!("givebank {talisman_alias} 1"));
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
            "confirm runecraft level before Start",
            Proof::Stat {
                id: RUNECRAFT_STAT,
                min: rc_level,
            },
        ),
        (
            "confirm no seeded essence in pack before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded selected runes in pack before Start",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong runes in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded talisman in pack before Start",
            Proof::ItemIdAtMost {
                id: talisman_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(runecraft_open_seed_bank(
        "open and acknowledge the exact unnoted essence seed bank",
        Proof::BankItemId {
            id: RUNE_ESSENCE_ID,
            count: RUNE_ESSENCE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact talisman seed bank",
        Proof::BankItemId {
            id: talisman_id,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted essence in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded selected runes in bank",
        Proof::BankItemIdAtMost {
            id: rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong runes in bank",
        Proof::BankItemIdAtMost {
            id: wrong_rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch script withdrawal of unnoted essence after Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch arrival at the selected mysterious ruins after Start",
            ruins_near,
        ),
        (
            "watch Runecraft XP from the selected craft",
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            },
        ),
        (
            "watch essence become the selected rune after altar entry",
            crafted,
        ),
        (
            "watch the withdrawn essence finish converting",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        ("watch portal exit back to the selected ruins", ruins_near),
        (
            "watch script-created runes enter a fresh bank",
            Proof::BankItemId {
                id: rune_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of runes after deposit",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "watch a restock of unnoted essence",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch the script close its runecraft bank",
            Proof::BankClosed,
        ),
        (
            "watch return to the selected ruins after restock",
            ruins_near,
        ),
        ("watch another exact selected rune after restock", crafted),
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
        proof: crafted,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(start_script),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const THIEVING_STAT: i32 = 17;
const HITPOINTS_STAT: i32 = 3;
const ARDY_CAKES_BALLAST_KNIVES: i32 = 22;
const CAKE_ID: i32 = 1891;
const BREAD_ID: i32 = 2309;
const CHOCOLATE_SLICE_ID: i32 = 1901;
const CHOCOLATE_CAKE_ID: i32 = 1897;

const ARDY_CAKES_STAND: WorldTile = WorldTile {
    x: 2668,
    z: 3312,
    level: 0,
};
const ARDY_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3286,
    level: 0,
};

const ARDY_CAKES_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_CAKES_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_THIEVER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_KNIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Knight of Ardougne"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

/// Six free slots at the Baker's stall stand. Flee, clues off. Script steals
/// cake/bread/chocolate slice with Thieving XP, deposits the acquired stock
/// when the pack is full, returns to STAND and steals again. Fight stays
/// pending. Cake 1891 is the sequential identity; chocolate cake 1897 is not
/// stall food.
fn ardy_cakes_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, "setstat hitpoints 40");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch arrival at the Ardougne bank after the stall fill",
            Proof::ArrivedNear {
                x: ARDY_BANK.x,
                z: ARDY_BANK.z,
                level: ARDY_BANK.level,
                radius: 6,
            },
        ),
        (
            "watch script-stolen cake enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: CAKE_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cake after deposit",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "watch return to the Baker's stall stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        ("watch the cake bank close after deposit", Proof::BankClosed),
        ("watch another exact Cake 1891 after return", cake),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_cakes",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: cake,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_INJECT),
            terminal_shot: Some("ardy_cakes"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill of the catching Guard.
/// Combat stats and a scimitar are prepared so the FightBack branch can land;
/// the Flee kite is not this cell. Catalog owns the Guard defeat witness.
fn ardy_cakes_fight_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_cakes_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: style_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_FIGHT_INJECT),
            terminal_shot: Some("ardy_cakes_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct ArdyThieverSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    thieving: i32,
}

fn ardy_thiever_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever",
        inject: ARDY_THIEVER_INJECT,
        thieving: 40,
    })
}

fn ardy_thiever_knight_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever_knight",
        inject: ARDY_THIEVER_KNIGHT_INJECT,
        thieving: 55,
    })
}

/// `guardResponse=Fight` on ArdyThiever: pickpocket coins, FightBack on catch,
/// then the same loot-count bank/return/further cycle as the Flee cell. Combat
/// kit is prepared before Start; catalog owns the Guard defeat + no-Flee gate.
fn ardy_thiever_fight_scenario() -> Scenario {
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 40");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 40,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_thiever_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(ARDY_THIEVER_FIGHT_INJECT),
            terminal_shot: Some("ardy_thiever_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack at the Guard/Knight stand. Flee, clues off, loot-count bank
/// at 1 slot. Script restocks one stall food, pickpockets coins with
/// Thieving XP, deposits those coins, returns to the stand and pickpockets
/// again. PeriodicBank Off is not the bank proof. Fight stays pending.
fn ardy_thiever_variant(spec: ArdyThieverSpec) -> Scenario {
    let ArdyThieverSpec {
        name,
        inject,
        thieving,
    } = spec;
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat thieving {thieving}"));
                cheat(c, "setstat hitpoints 40");
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
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
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const WOODCUTTING_STAT: i32 = 8;
const MINING_STAT: i32 = 14;
const MAGIC_LOGS_ID: i32 = 1513;
const NOTED_MAGIC_LOGS_ID: i32 = 1514;
const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
const MAGIC_SHORTBOW_ID: i32 = 861;
const MAGIC_LONGBOW_ID: i32 = 859;
const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
const KNIFE_ID: i32 = 946;
const RUNE_AXE_ID: i32 = 1359;
const MAGIC_TREE_ID: i32 = 1306;
/// One Rune axe plus 26 unstackable Knives leaves one product slot. The
/// frozen Gnome script preserves Knife as a tool during gear prep and bank trips.
const GNOME_BALLAST_KNIVES: i32 = 26;
const RUNE_PICKAXE_ID: i32 = 1275;
const NOTED_COAL_ID: i32 = 454;
/// A Rune pickaxe plus 26 unstackable Knives leaves one slot for Coal.
/// CoalTrucks keeps non-coal items when it empties the pack into a truck.
const COAL_BALLAST_KNIVES: i32 = 26;
/// With the ordinary prayer/ranged/magic defaults, 48 in each melee/HP stat
/// yields native combat level 55 without over-leveling the fixture.
const COAL_MELEE_LEVEL: i32 = 48;

const GNOME_SOUTH_BANK_MAGIC_STAND: WorldTile = WorldTile {
    x: 2433,
    z: 3409,
    level: 0,
};
const GNOME_SOUTH_BANK_MAGIC_TREE: WorldTile = WorldTile {
    x: 2432,
    z: 3410,
    level: 0,
};
const GNOME_BANK_STAND: WorldTile = WorldTile {
    x: 2445,
    z: 3425,
    level: 1,
};
const GNOME_BANK_STAIR_SOUTH: WorldTile = WorldTile {
    x: 2444,
    z: 3416,
    level: 0,
};
const COAL_MINE: WorldTile = WorldTile {
    x: 2582,
    z: 3481,
    level: 0,
};
const COAL_MINE_TRUCK_STAND: WorldTile = WorldTile {
    x: 2575,
    z: 3486,
    level: 0,
};

const GNOME_CHOP_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(false),
}];

const GNOME_FLETCH_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(true),
}];

/// One free product slot at the south-bank Magic tree. fletchLogs off. Seed WC 75, Rune
/// axe 1359, and retained nonproduct Knife ballast. The script chops one
/// magic log 1513 with Woodcutting XP, deposits it at the upstairs gnome
/// booth, returns to ground and chops again. This qualifies the resource
/// cycle, not ordinary 28-slot throughput. Death recovery stays out.
fn gnome_chop_scenario() -> Scenario {
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_xp = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed woodcutting, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
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
    for (step_name, arm) in [
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_xp,
        ),
        ("watch exact Magic logs 1513 chopped after Start", logs),
        (
            "watch arrival at the upstairs gnome booth after the log fill",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAND.x,
                z: GNOME_BANK_STAND.z,
                level: GNOME_BANK_STAND.level,
                radius: 8,
            },
        ),
        (
            "watch script-chopped magic logs enter a fresh gnome bank",
            Proof::BankItemId {
                id: MAGIC_LOGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of magic logs after deposit",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
        ),
        (
            "watch the gnome log bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Magic logs 1513 after return", logs),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "gnome_chop",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_CHOP_INJECT),
            terminal_shot: Some("gnome_chop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct GnomeFletchSpec {
    name: &'static str,
    fletching: i32,
    fletching_max: Option<i32>,
    product_id: i32,
}

fn gnome_fletch_short_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_short",
        fletching: 80,
        fletching_max: Some(84),
        product_id: UNSTRUNG_MAGIC_SHORTBOW_ID,
    })
}

fn gnome_fletch_long_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_long",
        fletching: 85,
        fletching_max: None,
        product_id: UNSTRUNG_MAGIC_LONGBOW_ID,
    })
}

/// fletchLogs on. Seed WC 75, Fletching 80/85, Rune axe, and 26 retained
/// Knives so one script-chopped log fills the pack. The script consumes it
/// into exact unstrung 72/70 with Fletching XP, deposits the bow upstairs,
/// returns to ground and chops again. This is cycle qualification, not
/// ordinary capacity proof. Missing Knife is a stop, not a pass; strung
/// 861/859 are not the unstrung product.
fn gnome_fletch_variant(spec: GnomeFletchSpec) -> Scenario {
    let GnomeFletchSpec {
        name,
        fletching,
        fletching_max,
        product_id,
    } = spec;
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_wc = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let first_fletch = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name:
            "seed woodcutting, fletching, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, &format!("setstat fletching {fletching}"));
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
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
    let mut seed_arms = vec![
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm prepared Fletching before Start",
            Proof::Stat {
                id: FLETCHING_STAT,
                min: fletching,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ];
    if let Some(max) = fletching_max {
        seed_arms.insert(
            2,
            (
                "confirm Fletching below longbow 85 before Start",
                Proof::StatAtMost {
                    id: FLETCHING_STAT,
                    max,
                },
            ),
        );
    }
    for (step_name, arm) in seed_arms {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_wc,
        ),
        ("watch exact Magic logs 1513 chopped after Start", logs),
        ("watch Fletching XP after Start", first_fletch),
        (
            "watch exact unstrung magic bow after logs are consumed",
            product,
        ),
        (
            "watch script-fletched bows enter a fresh gnome bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of unstrung bows after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
        ),
        (
            "watch the gnome fletch bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Magic logs 1513 after return", logs),
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
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_FLETCH_INJECT),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Seed Mining 60, ordinary combat-55 melee stats, Rune pickaxe 1275, and
/// 26 retained nonproduct Knives on the safe initial tile. The one free slot
/// makes the first mined Coal fill the pack. Observe real mining XP and exact
/// coal 453, then a mine-truck deposit (pack empty of coal at the truck stand,
/// not a Seers bank), then further mining.
/// Filling truck 120 then Seers haul/bank/return cannot fit
/// SCRIPT_GOLD_DEADLINE 180s from an empty truck; no truck-content seed
/// primitive exists. Death/combat recovery is not this core.
fn coal_trucks_scenario() -> Scenario {
    let stand = COAL_MINE;
    let first_xp = Proof::StatXpGain {
        id: MINING_STAT,
        min: 1,
    };
    let coal = Proof::ItemId {
        id: COAL_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed mining 60, combat-safe melee stats, Rune pickaxe and 26-Knife ballast",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat mining 60");
                cheat(c, &format!("setstat attack {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat strength {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat defence {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COAL_MELEE_LEVEL}"));
                cheat(c, "give rune_pickaxe 1");
                cheat(c, &format!("give knife {COAL_BALLAST_KNIVES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Attack 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Strength 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Defence 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 1,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 3,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Mining 60 before Start",
            Proof::Stat {
                id: MINING_STAT,
                min: 60,
            },
        ),
        (
            "confirm Rune pickaxe 1275 before Start",
            Proof::ItemId {
                id: RUNE_PICKAXE_ID,
                count: 1,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives and one available slot before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded coal in pack before Start",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted coal in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_COAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(Step {
        name: "teleport into the giant-bat mine after combat readiness is acknowledged",
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
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Mining XP from coal rocks after Start", first_xp),
        ("watch exact Coal 453 mined after Start", coal),
        (
            "watch arrival at the mine coal truck after the pack fill",
            Proof::ArrivedNear {
                x: COAL_MINE_TRUCK_STAND.x,
                z: COAL_MINE_TRUCK_STAND.z,
                level: COAL_MINE_TRUCK_STAND.level,
                radius: 4,
            },
        ),
        (
            "watch the pack empty of coal after the mine-truck deposit",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        ("watch another exact Coal 453 after the truck deposit", coal),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "coal_trucks",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coal,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CoalTrucks"),
            terminal_shot: Some("coal_trucks"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const COOKING_STAT: i32 = 7;
const RAW_SALMON_ID: i32 = 331;
const SALMON_ID: i32 = 329;
const NOTED_RAW_SALMON_ID: i32 = 332;
const NOTED_SALMON_ID: i32 = 330;
const RAW_LOBSTER_ID: i32 = 377;
const LOBSTER_ID: i32 = 379;
const NOTED_RAW_LOBSTER_ID: i32 = 378;
const NOTED_LOBSTER_ID: i32 = 380;
const BURNT_LOBSTER_ID: i32 = 381;
const BURNT_FISH_1_ID: i32 = 323;
const BURNT_FISH_2_ID: i32 = 343;
const NOTED_COPPER_ORE_ID: i32 = 437;
const NOTED_TIN_ORE_ID: i32 = 439;
const NOTED_IRON_ORE_ID: i32 = 441;
const NOTED_BRONZE_BAR_ID: i32 = 2350;
const NOTED_STEEL_BAR_ID: i32 = 2354;
const NOTED_FLAX_ID: i32 = 1780;
const NOTED_BOW_STRING_ID: i32 = 1778;
const BALL_OF_WOOL_ID: i32 = 1759;
const COOKING_FIXTURE_LEVEL: i32 = 80;
const COOK_RAW_SEED: i32 = 56;
const SMELT_ORE_SEED: i32 = 56;
const STEEL_COAL_SEED: i32 = 112;
const FLAX_SPIN_SEED: i32 = 56;

const CATHERBY_BANK: WorldTile = WorldTile {
    x: 2809,
    z: 3441,
    level: 0,
};
const CATHERBY_RANGE_STAND: WorldTile = WorldTile {
    x: 2817,
    z: 3443,
    level: 0,
};
const AL_KHARID_FURNACE: WorldTile = WorldTile {
    x: 3275,
    z: 3185,
    level: 0,
};
const FLAX_SPINNER_BANK: WorldTile = WorldTile {
    x: 2722,
    z: 3493,
    level: 0,
};
const FLAX_SPINNER_WHEEL: WorldTile = WorldTile {
    x: 2711,
    z: 3471,
    level: 1,
};
/// FlaxAIO BANK_STAND; the spinner booth seed is 2722,3493,0.
const FLAX_AIO_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3493,
    level: 0,
};
const RED_SPIDERS_EGGS_ID: i32 = 223;
const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
const NOTED_EYE_OF_NEWT_ID: i32 = 222;
const HERBLORE_EGG_FOOD_SEED: i32 = 50;
const HERBLORE_NEWT_COIN_SEED: i32 = 5000;
const EGG_FIELD: WorldTile = WorldTile {
    x: 3120,
    z: 9952,
    level: 0,
};
const EDGEVILLE_BANK: WorldTile = WorldTile {
    x: 3094,
    z: 3493,
    level: 0,
};
/// Selected 274/289 packs both contain this Edgeville booth as id 2213 with
/// native booth op2 (`Use-quickly`). Keep the walk stand separate from it.
const EDGEVILLE_BANK_BOOTH: WorldTile = WorldTile {
    x: 3096,
    z: 3493,
    level: 0,
};
const EDGEVILLE_BANK_BOOTH_ID: i32 = 2213;
const BETTY_SHOP: WorldTile = WorldTile {
    x: 3012,
    z: 3259,
    level: 0,
};
const DRAYNOR_BANK: WorldTile = WorldTile {
    x: 3093,
    z: 3243,
    level: 0,
};

const FLAX_AIO_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
const FLAX_AIO_PICK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(false),
    },
];
const FLAX_AIO_SPIN_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
const HERBLORE_EGGS_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "secondary",
    value: ScriptInjectValue::Str("Red spiders' eggs"),
}];
const HERBLORE_NEWT_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "secondary",
    value: ScriptInjectValue::Str("Eye of newt"),
}];

const TROUT_ID: i32 = 333;
const COMBAT_SCIMITAR_ID: i32 = 1331;
const MIND_RUNE_ID: i32 = 558;
const BIG_BONES_ID: i32 = 532;
const NOTED_BIG_BONES_ID: i32 = 533;
const LIMPWURT_ROOT_ID: i32 = 225;
const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
const LAW_RUNE_ID: i32 = 563;
const BONES_ID: i32 = 526;
const NOTED_BONES_ID: i32 = 527;
const NOTED_HERB_ID: i32 = 200;
const CHAOS_DRUID_FOOD: i32 = 12;
const MOSS_GIANT_FOOD: i32 = 10;
const HILL_GIANT_FOOD: i32 = 8;
const AUTO_FIGHTER_FOOD: i32 = 8;
/// Bank-cell preparation: MossGiant only banks once the pack's food is gone, so
/// the bank cell carries a shortfall instead of a full pack; ChaosDruidKiller's
/// own `tripPrepared` needs `foodWithdraw` (12) in the field, so 8 forces its
/// declared `prepare-trip` end.
const MOSS_GIANT_BANK_FOOD: i32 = 2;
const CHAOS_DRUID_BANK_FOOD: i32 = 8;
/// Restock lines the cards themselves withdraw to (MossGiant's declared
/// `foodWithdraw` default 20, AutoFighter's 10, HillGiant's 12).
const MOSS_GIANT_BANK_RESTOCK: i32 = 20;
const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
const HILL_GIANT_BANK_RESTOCK: i32 = 4;
const AUTO_FIGHTER_MAGE_LEVEL: i32 = 13;
const AUTO_FIGHTER_MAGE_CASTS: i32 = 150;
const AUTO_FIGHTER_MAGE_AIR_RUNES: i32 = AUTO_FIGHTER_MAGE_CASTS * 2;
const AUTOCAST_MAGIC_VARP: i32 = 108;
const AUTOCAST_ARMED_VALUE: i32 = 3;
const RANGED_STAT: i32 = 4;
const COMBAT_MODE_VARP: i32 = 43;
const RAPID_COMBAT_MODE: i32 = 1;
const MAPLE_SHORTBOW_ID: i32 = 853;
const BRONZE_ARROW_ID: i32 = 882;
const RANGE_AMMO: i32 = 200;
const ROCK_CRAB_FOOD: i32 = 8;
const GREEN_DRAGON_FOOD: i32 = 12;
const FIRE_GIANT_FOOD: i32 = 12;
const COMBAT_ATTACK_LEVEL: i32 = 40;
const RUNE_SCIMITAR_ID: i32 = 1333;
const DRAGONFIRE_SHIELD_ID: i32 = 1540;
const DRAGON_DAGGER_ID: i32 = 1215;
const DRAGON_DAGGER_ATTACK_LEVEL: i32 = 60;
const SPECIAL_ENERGY_VARP: i32 = 300;
const DRAGON_DAGGER_SPECIAL_COST: i32 = 250;
const SUPER_ATTACK_3_ID: i32 = 145;
const SUPER_ATTACK_2_ID: i32 = 147;
const SUPER_STRENGTH_3_ID: i32 = 157;
const SUPER_STRENGTH_2_ID: i32 = 159;
const BRASS_KEY_ID: i32 = 983;
const DRAGON_BONES_ID: i32 = 536;
const NOTED_DRAGON_BONES_ID: i32 = 537;
const GREEN_DRAGONHIDE_ID: i32 = 1753;
const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
const GLARIALS_AMULET_ID: i32 = 295;
const ROPE_ID: i32 = 954;
const CASKET_ID: i32 = 405;
const NOTED_CASKET_ID: i32 = 406;
const NOTED_UNCUT_SAPPHIRE_ID: i32 = 1624;
const STEEL_ARROW_ID: i32 = 886;
const BODY_TALISMAN_ID: i32 = 1446;
const BLOOD_RUNE_ID: i32 = 565;
const CHAOS_RUNE_ID: i32 = 562;
/// The six verifiable Guard drops the AutoFighter bank cell's injected loot
/// list holds (`loot=[iron ore, steel arrow, body talisman, blood/chaos/nature
/// rune]`) and the class the host core's `GUARD_DROP_IDS` deposits. A
/// clue-only or junk drop is not one of these.
const GUARD_DROP_IDS: [i32; 6] = [
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];

const CHAOS_DRUID_FIELD: WorldTile = WorldTile {
    x: 3110,
    z: 9936,
    level: 0,
};
const CHAOS_DRUID_TOWER_FIELD: WorldTile = WorldTile {
    x: 2562,
    z: 3356,
    level: 0,
};
const CHAOS_DRUID_YANILLE_FIELD: WorldTile = WorldTile {
    x: 2580,
    z: 9501,
    level: 0,
};
const MOSS_GIANT_SAFESPOT: WorldTile = WorldTile {
    x: 2553,
    z: 3406,
    level: 0,
};
const HILL_GIANT_PIT: WorldTile = WorldTile {
    x: 3110,
    z: 9832,
    level: 0,
};
const ROCK_CRAB_SPOT: WorldTile = WorldTile {
    x: 2704,
    z: 3726,
    level: 0,
};
/// Default source-script reset tile: outside the wake radius but in the
/// loaded rock-crab field, so dormant `Rocks` can be observed before Start.
const ROCK_CRAB_SAFE_STAND: WorldTile = WorldTile {
    x: 2712,
    z: 3688,
    level: 0,
};
const GREEN_DRAGON_FIELD: WorldTile = WorldTile {
    x: 3096,
    z: 3814,
    level: 0,
};
const FIRE_GIANT_ROOM: WorldTile = WorldTile {
    x: 2575,
    z: 9893,
    level: 0,
};

const CHAOS_DRUID_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Edgeville Dungeon"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const CHAOS_DRUID_TOWER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Chaos Druid Tower"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const CHAOS_DRUID_YANILLE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Yanille Dungeon"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const MOSS_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const HILL_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const AUTO_FIGHTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const AUTO_FIGHTER_MAGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("mage"),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Fire Strike"),
    },
    ScriptSettingInject {
        id: "runesWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_MAGE_CASTS as f64),
    },
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const AUTO_FIGHTER_RANGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("range"),
    },
    ScriptSettingInject {
        id: "rangeStyle",
        value: ScriptInjectValue::Str("rapid"),
    },
    ScriptSettingInject {
        id: "ammo",
        value: ScriptInjectValue::Str("Bronze arrow"),
    },
    ScriptSettingInject {
        id: "ammoWithdraw",
        value: ScriptInjectValue::Num(RANGE_AMMO as f64),
    },
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const ROCK_CRAB_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
];
const ROCK_CRAB_RANGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("range"),
    },
    ScriptSettingInject {
        id: "rangeStyle",
        value: ScriptInjectValue::Str("rapid"),
    },
    ScriptSettingInject {
        id: "bow",
        value: ScriptInjectValue::Str("Maple shortbow"),
    },
    ScriptSettingInject {
        id: "ammo",
        value: ScriptInjectValue::Str("Bronze arrow"),
    },
    ScriptSettingInject {
        id: "ammoWithdraw",
        value: ScriptInjectValue::Num(RANGE_AMMO as f64),
    },
    ScriptSettingInject {
        id: "minStack",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "collectRange",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
];
const GREEN_DRAGON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_SPECIAL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Dragon dagger"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_POTIONS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const FIRE_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "escapeTele",
        value: ScriptInjectValue::Str("Barrel (free)"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const ARDY_FIGHTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];
const CHAOS_DRUID_LOOT_EMPTY: &[i32] = &[
    UNIDENTIFIED_GUAM_ID,
    NATURE_RUNE_ID,
    LAW_RUNE_ID,
    NOTED_HERB_ID,
];
const MOSS_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
const HILL_GIANT_LOOT_EMPTY: &[i32] = &[
    BIG_BONES_ID,
    NOTED_BIG_BONES_ID,
    LIMPWURT_ROOT_ID,
    NOTED_LIMPWURT_ROOT_ID,
];
const AUTO_FIGHTER_LOOT_EMPTY: &[i32] = &[BONES_ID, NOTED_BONES_ID];
const ROCK_CRAB_LOOT_EMPTY: &[i32] = &[
    UNCUT_SAPPHIRE_ID,
    NOTED_UNCUT_SAPPHIRE_ID,
    CASKET_ID,
    NOTED_CASKET_ID,
];
const GREEN_DRAGON_LOOT_EMPTY: &[i32] = &[
    DRAGON_BONES_ID,
    NOTED_DRAGON_BONES_ID,
    GREEN_DRAGONHIDE_ID,
    NOTED_GREEN_DRAGONHIDE_ID,
];
const FIRE_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
const ARDY_FIGHTER_LOOT_EMPTY: &[i32] = &[CAKE_ID, BREAD_ID, CHOCOLATE_SLICE_ID, CHOCOLATE_CAKE_ID];

const COOK_BOT_SALMON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw salmon"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];

const COOK_BOT_LOBSTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw lobster"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];

const SMELTER_BOT_BRONZE_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SMELTER_BOT_STEEL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Steel"),
}];

fn cook_bot_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot",
        inject: COOK_BOT_SALMON_INJECT,
        raw_alias: "raw_salmon",
        raw_id: RAW_SALMON_ID,
        product_id: SALMON_ID,
        wrong_product_id: LOBSTER_ID,
        noted_raw_id: NOTED_RAW_SALMON_ID,
        noted_product_id: NOTED_SALMON_ID,
    })
}

fn cook_bot_lobster_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot_lobster",
        inject: COOK_BOT_LOBSTER_INJECT,
        raw_alias: "raw_lobster",
        raw_id: RAW_LOBSTER_ID,
        product_id: LOBSTER_ID,
        wrong_product_id: SALMON_ID,
        noted_raw_id: NOTED_RAW_LOBSTER_ID,
        noted_product_id: NOTED_LOBSTER_ID,
    })
}

struct CookSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    raw_alias: &'static str,
    raw_id: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_raw_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Catherby bank. Banked raw fish, never cooked/burnt/noted.
/// Cooking 80 is a fixture seed so ordinary burn randomness does not replace
/// the product contract; the script encodes no cook level. Range only — Fire
/// stays behind native fire work. Script withdraws, cooks on the Catherby
/// Range, deposits the exact product, restocks raw, returns and cooks again.
fn cook_bot_variant(spec: CookSpec) -> Scenario {
    let CookSpec {
        name,
        inject,
        raw_alias,
        raw_id,
        product_id,
        wrong_product_id,
        noted_raw_id,
        noted_product_id,
    } = spec;
    let range = Proof::ArrivedNear {
        x: CATHERBY_RANGE_STAND.x,
        z: CATHERBY_RANGE_STAND.z,
        level: CATHERBY_RANGE_STAND.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = CATHERBY_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cooking, banked raw fish, and tele to Catherby bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat cooking {COOKING_FIXTURE_LEVEL}"));
                cheat(c, &format!("givebank {raw_alias} {COOK_RAW_SEED}"));
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
            "confirm Cooking 80 before Start",
            Proof::Stat {
                id: COOKING_STAT,
                min: COOKING_FIXTURE_LEVEL,
            },
        ),
        (
            "confirm no seeded raw fish in pack before Start",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded cooked product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong cooked fish in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish 323 in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_1_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_2_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted raw in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted product in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact raw-fish seed bank",
        Proof::BankItemId {
            id: raw_id,
            count: COOK_RAW_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded cooked product in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted raw in bank",
        Proof::BankItemIdAtMost {
            id: noted_raw_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Catherby Range after Start", range),
        (
            "watch Cooking XP from the Catherby Range after Start",
            Proof::StatXpGain {
                id: COOKING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted cooked fish after Start", product),
        (
            "watch the withdrawn raw finish converting",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "watch script-cooked fish enter a fresh Catherby bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cooked fish after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact raw fish",
            Proof::ItemId {
                id: raw_id,
                count: 1,
            },
        ),
        ("watch the script close its cook bank", Proof::BankClosed),
        ("watch return to the Catherby Range after restock", range),
        ("watch another exact cooked fish after restock", product),
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
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CookBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn smelter_bot_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot",
        inject: SMELTER_BOT_BRONZE_INJECT,
        smithing: 1,
        primary_alias: "copper_ore",
        primary_id: COPPER_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "tin_ore",
        secondary_id: TIN_ORE_ID,
        secondary_seed: SMELT_ORE_SEED,
        product_id: BRONZE_BAR_ID,
        wrong_product_id: STEEL_BAR_ID,
        noted_primary_id: NOTED_COPPER_ORE_ID,
        noted_secondary_id: NOTED_TIN_ORE_ID,
        noted_product_id: NOTED_BRONZE_BAR_ID,
    })
}

fn smelter_bot_steel_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot_steel",
        inject: SMELTER_BOT_STEEL_INJECT,
        smithing: 30,
        primary_alias: "iron_ore",
        primary_id: IRON_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "coal",
        secondary_id: COAL_ID,
        secondary_seed: STEEL_COAL_SEED,
        product_id: STEEL_BAR_ID,
        wrong_product_id: BRONZE_BAR_ID,
        noted_primary_id: NOTED_IRON_ORE_ID,
        noted_secondary_id: NOTED_COAL_ID,
        noted_product_id: NOTED_STEEL_BAR_ID,
    })
}

struct SmelterSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    primary_alias: &'static str,
    primary_id: i32,
    primary_seed: i32,
    secondary_alias: &'static str,
    secondary_id: i32,
    secondary_seed: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_primary_id: i32,
    noted_secondary_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Al-Kharid bank. Banked ores, never bars. Script withdraws
/// the recipe, smelts at the real furnace, deposits the exact bar, restocks
/// ore, returns and smelts again. Smithing main panel is not this hop.
fn smelter_bot_variant(spec: SmelterSpec) -> Scenario {
    let SmelterSpec {
        name,
        inject,
        smithing,
        primary_alias,
        primary_id,
        primary_seed,
        secondary_alias,
        secondary_id,
        secondary_seed,
        product_id,
        wrong_product_id,
        noted_primary_id,
        noted_secondary_id,
        noted_product_id,
    } = spec;
    let furnace = Proof::ArrivedNear {
        x: AL_KHARID_FURNACE.x,
        z: AL_KHARID_FURNACE.z,
        level: AL_KHARID_FURNACE.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed smithing, banked ores, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat smithing {smithing}"));
                cheat(c, &format!("givebank {primary_alias} {primary_seed}"));
                cheat(c, &format!("givebank {secondary_alias} {secondary_seed}"));
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
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
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
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong bars in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
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
        (
            "confirm no seeded noted product in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact primary-ore seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: primary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary-ore seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: secondary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bars in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted primary in bank",
        Proof::BankItemIdAtMost {
            id: noted_primary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted secondary in bank",
        Proof::BankItemIdAtMost {
            id: noted_secondary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Al-Kharid furnace after Start",
            furnace,
        ),
        (
            "watch Smithing XP from the furnace after Start",
            Proof::StatXpGain {
                id: SMITHING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bar after Start", product),
        (
            "watch the withdrawn primary ore finish converting",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "watch script-smelted bars enter a fresh Al-Kharid bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bars after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ),
        ("watch the script close its smelt bank", Proof::BankClosed),
        (
            "watch return to the Al-Kharid furnace after restock",
            furnace,
        ),
        ("watch another exact bar after restock", product),
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
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("SmelterBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack at the Seers flax bank. Banked flax 1779, never bow string.
/// Script withdraws, climbs to the wheel, spins Flax into 1777 with Crafting
/// XP, deposits, restocks, returns upstairs and spins again. Wool is not this
/// core.
fn flax_spinner_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_SPINNER_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to the Seers flax bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 1");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Crafting 1 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_spinner",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxSpinner"),
            terminal_shot: Some("flax_spinner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn flax_aio_empty_pack_confirms() -> Vec<(&'static str, Proof)> {
    vec![
        (
            "confirm Crafting 1 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ]
}

/// Empty pack at the Seers flax field. FlaxAIO own script, both flags on.
/// Pick 1779, climb, makeX 1779→1777 with Crafting XP, deposit strings,
/// closed return to the field, further Pick. Not FlaxPicker+FlaxSpinner.
fn flax_aio_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let flax = Proof::ItemId {
        id: FLAX_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting and tele to the Seers flax field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 1");
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
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact flax 1779 from the Seers field after Start",
            flax,
        ),
        (
            "watch arrival at the upstairs spinning wheel after picking",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the picked flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        ("watch the script close its flax bank", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        ("watch further exact flax after return", flax),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: flax,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_INJECT),
            terminal_shot: Some("flax_aio"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO pick-only. Full flax pack, deposit 1779, closed return, further
/// Pick. Spun 1777 must not qualify.
fn flax_aio_pick_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the Seers flax field before Start",
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
    for (step_name, arm) in [
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
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
        (
            "watch the pack empty of flax after deposit",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
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
        name: "flax_aio_pick",
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
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_PICK_INJECT),
            terminal_shot: Some("flax_aio_pick"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO spin-only. Banked flax at FlaxAIO's own booth stand, wheel
/// conversion+XP, string deposit, restock, closed return upstairs, further
/// spin. Wool is not this core.
fn flax_aio_spin_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_AIO_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to FlaxAIO's Seers bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 1");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio_spin",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_SPIN_INJECT),
            terminal_shot: Some("flax_aio_spin"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn herblore_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(|c, snapshot| {
                match Interactions::new(snapshot, c)
                    .open_booth_at(EDGEVILLE_BANK_BOOTH, EDGEVILLE_BANK_BOOTH_ID)
                {
                    SendResult::Sent { .. } => true,
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { reason, .. } => {
                        eprintln!("[scenario] exact Edgeville booth send refused: {reason:?}");
                        false
                    }
                }
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn herblore_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Edgeville booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: EDGEVILLE_BANK_BOOTH_ID,
            x: EDGEVILLE_BANK_BOOTH.x,
            z: EDGEVILLE_BANK_BOOTH.z,
            level: EDGEVILLE_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}

/// HerbloreSecondaries default Red spiders' eggs. Empty pack at the
/// Edgeville dungeon field, lobster food banked only. Ground Take 223,
/// deposit, closed return, further Take. Eggs are not given.
fn herblore_secondaries_scenario() -> Scenario {
    let field = EGG_FIELD;
    let bank = EDGEVILLE_BANK;
    let eggs = Proof::ItemId {
        id: RED_SPIDERS_EGGS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed banked lobster at Edgeville and tele to the egg field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank lobster {HERBLORE_EGG_FOOD_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(herblore_seed_bank_readiness());
    steps.push(herblore_open_seed_bank(
        "open and acknowledge the exact lobster food seed bank",
        Proof::BankItemId {
            id: LOBSTER_ID,
            count: HERBLORE_EGG_FOOD_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded red spiders' eggs in bank",
        Proof::BankItemIdAtMost {
            id: RED_SPIDERS_EGGS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted eggs in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_RED_SPIDERS_EGGS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded eye of newt in bank",
        Proof::BankItemIdAtMost {
            id: EYE_OF_NEWT_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "tele to the Edgeville dungeon egg field before Start",
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
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact red spiders' eggs 223 from the ground after Start",
            eggs,
        ),
        (
            "watch script-taken eggs enter a fresh Edgeville bank",
            Proof::BankItemId {
                id: RED_SPIDERS_EGGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of eggs after deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        ("watch the script close its egg bank", Proof::BankClosed),
        (
            "watch return to the egg field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 14,
            },
        ),
        ("watch further exact eggs after return", eggs),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "herblore_secondaries",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: eggs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_EGGS_INJECT),
            terminal_shot: Some("herblore_secondaries"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// HerbloreSecondaries Eye of newt shop branch. Banked coins, Betty stand.
/// Distinct from ground eggs. LIVE still waits on Shop.buy publication.
fn herblore_secondaries_newt_scenario() -> Scenario {
    let shop = BETTY_SHOP;
    let bank = DRAYNOR_BANK;
    let newt = Proof::ItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed banked coins at Draynor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank coins {HERBLORE_NEWT_COIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact coin seed bank",
        Proof::BankItemId {
            id: COINS_ID,
            count: HERBLORE_NEWT_COIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded eye of newt in bank",
        Proof::BankItemIdAtMost {
            id: EYE_OF_NEWT_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted newt in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_EYE_OF_NEWT_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded red spiders' eggs in bank",
        Proof::BankItemIdAtMost {
            id: RED_SPIDERS_EGGS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "tele to Betty's shop stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(shop.level, shop.x, shop.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch exact eye of newt 221 after Start", newt),
        (
            "watch script-bought newt enter a fresh Draynor bank",
            Proof::BankItemId {
                id: EYE_OF_NEWT_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of newt after deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        ("watch the script close its newt bank", Proof::BankClosed),
        (
            "watch return to Betty after banking",
            Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
        ),
        ("watch further exact newt after return", newt),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "herblore_secondaries_newt",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: newt,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_NEWT_INJECT),
            terminal_shot: Some("herblore_secondaries_newt"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Shared melee-core seed: legal stats, food, ordinary weapon, empty loot,
/// then Start. Banking policy is explicit in the inject; these cells are not
/// bank-roundtrip proof. Scenario watch is selected-style XP; catalog adds
/// two engagements, a verified defeat, and exact loot where required.
struct CombatCorePlan {
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    wear_id: Option<i32>,
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    complete_quest: Option<&'static str>,
    thieving: i32,
    agility: i32,
}

fn combat_core_scenario(plan: CombatCorePlan) -> Scenario {
    let CombatCorePlan {
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        wear_id,
        loot_empty,
        inject,
        complete_quest,
        thieving,
        agility,
    } = plan;
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    if let Some(quest) = complete_quest {
        steps.push(Step {
            name: "complete required quests before Start",
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
        });
        steps.push(Step {
            name: "answer the quest-seed dialogs until the journal is green",
            kind: StepKind::DrainDialogs { choice: 1 },
            wait: Wait {
                arm: Proof::QuestDone { name: quest },
                budget_ticks: 600,
            },
        });
    }
    steps.push(Step {
        name: "prepare melee stats, food and gear on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                if agility > 0 {
                    cheat(c, &format!("setstat agility {agility}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {weapon_alias} 1"));
                if food_count > 0 {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Attack 40",
        Proof::Stat {
            id: 0,
            min: COMBAT_ATTACK_LEVEL,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Strength 40",
        Proof::Stat {
            id: STRENGTH_STAT,
            min: COMBAT_ATTACK_LEVEL,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Hitpoints 40",
        Proof::Stat {
            id: 3,
            min: COMBAT_ATTACK_LEVEL,
        },
    ));
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if agility > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Agility before Start",
            Proof::Stat {
                id: AGILITY_STAT,
                min: agility,
            },
        ));
    }
    if food_count > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared food in pack before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge prepared weapon before Start",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    if let Some(id) = wear_id {
        steps.push(Step {
            name: "wear and acknowledge Dragonfire shield before hostile-field teleport",
            kind: StepKind::Perform {
                send: Box::new(move |c, snapshot| {
                    matches!(
                        Interactions::new(snapshot, c).wear(id),
                        SendResult::Sent { .. }
                    )
                }),
            },
            wait: Wait {
                arm: Proof::EquipmentId { id },
                budget_ticks: 200,
            },
        });
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded combat loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
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
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch Strength XP from the selected melee style after Start",
        xp,
    ));
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
            start_script: Some(card),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn wear_combat_item_step(name: &'static str, id: i32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).wear(id),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id },
            budget_ticks: 200,
        },
    }
}

/// Prepared bow/quiver branch. The frozen script owns combat-mode selection,
/// target engagement, ammunition use and (for RockCrab) projectile recovery.
#[allow(clippy::too_many_arguments)]
fn combat_range_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    inject: &'static [ScriptSettingInject],
    dormant_rocks: bool,
) -> Scenario {
    let ranged_xp = Proof::StatXpGain {
        id: RANGED_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Ranged stats, food, bow and arrows on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat ranged {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "~clearinv");
                cheat(c, "give maple_shortbow 1");
                cheat(c, &format!("give bronze_arrow {RANGE_AMMO}"));
                cheat(c, &format!("give {food_alias} {food_count}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: RANGED_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Hitpoints before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge prepared range food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ),
        (
            "acknowledge Maple shortbow before wielding",
            Proof::ItemId {
                id: MAPLE_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "acknowledge exact Bronze arrow stack before equipping",
            Proof::ItemId {
                id: BRONZE_ARROW_ID,
                count: RANGE_AMMO,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Maple shortbow before hostile-field teleport",
        MAPLE_SHORTBOW_ID,
    ));
    steps.push(wear_combat_item_step(
        "equip and acknowledge Bronze arrows before hostile-field teleport",
        BRONZE_ARROW_ID,
    ));
    steps.push(Step {
        name: "teleport into the ranged field only after preparation is acknowledged",
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
                radius,
            },
            budget_ticks: 200,
        },
    });
    if dormant_rocks {
        steps.push(bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ));
    }
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the frozen script select rapid ranged mode",
        Proof::Varp {
            id: COMBAT_MODE_VARP,
            min: RAPID_COMBAT_MODE,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Ranged XP from actual ammunition combat after Start",
        ranged_xp,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: ranged_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// ChaosDruidKiller Edgeville dungeon core. Own bank is not this cell.
/// Style index 1 on the wielded weapon; lobster x12 so tripPrepared holds.
fn chaos_druid_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_FIELD,
        radius: 14,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// Chaos Druid Tower surface camp. Same Chaos druid identity and Herb/Law/Nature
/// loot as Edgeville; Thieving 46 is the door/approach prerequisite, prepared and
/// acknowledged before Start. Banking is not this cell.
fn chaos_druid_tower_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid_tower",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_TOWER_FIELD,
        radius: 4,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_TOWER_INJECT,
        complete_quest: None,
        thieving: 46,
        agility: 0,
    })
}

/// Yanille Dungeon warrior room. Target display is Chaos druid warrior; Agility 40
/// is the room prerequisite. Approach web/ledge is not this cell.
fn chaos_druid_yanille_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid_yanille",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_YANILLE_FIELD,
        radius: 8,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_YANILLE_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 40,
    })
}

/// MossGiant default melee at the safespot. Big bones 532 is catalog loot.
/// DeathRecovery stays idle. Banking is not this cell.
fn moss_giant_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "moss_giant",
        card: "MossGiant",
        tele: MOSS_GIANT_SAFESPOT,
        radius: 10,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: MOSS_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: MOSS_GIANT_LOOT_EMPTY,
        inject: MOSS_GIANT_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// HillGiant default melee in the pit. Target display is Giant. The Brass key
/// is prepared because this inside-pit cell does not qualify the key-fetch or
/// entrance branch. Blank weapon. DeathRecovery and banking stay idle.
fn hill_giant_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "hill_giant",
        card: "HillGiant",
        tele: HILL_GIANT_PIT,
        radius: 16,
        food_alias: "trout",
        food_id: TROUT_ID,
        food_count: HILL_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        wear_id: None,
        loot_empty: HILL_GIANT_LOOT_EMPTY,
        inject: HILL_GIANT_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// AutoFighter Guard at Start position. banking=None, clues/special off.
/// Gem-table loot is not required. DeathRecovery stays idle.
fn auto_fighter_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "auto_fighter",
        card: "AutoFighter",
        tele: ARDOUGNE_GUARD,
        radius: 8,
        food_alias: "trout",
        food_id: TROUT_ID,
        food_count: AUTO_FIGHTER_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: AUTO_FIGHTER_LOOT_EMPTY,
        inject: AUTO_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// AutoFighter's supported Fire Strike branch. Staff and exact cast supplies
/// are prepared and acknowledged on a safe tile before the Guard teleport.
/// Post-Start arms require native autocast state, Magic XP, and both paid rune
/// types to be consumed; the catalog witness adds death and further-combat proof.
fn auto_fighter_mage_scenario() -> Scenario {
    let magic_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Fire Strike stats, food, staff and runes on the safe tile",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &format!("setstat magic {AUTO_FIGHTER_MAGE_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "~clearinv");
                cheat(c, "give staff_of_fire 1");
                cheat(c, &format!("give trout {AUTO_FIGHTER_FOOD}"));
                cheat(c, &format!("give mindrune {AUTO_FIGHTER_MAGE_CASTS}"));
                cheat(c, &format!("give airrune {AUTO_FIGHTER_MAGE_AIR_RUNES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: MAGIC_STAT,
                min: AUTO_FIGHTER_MAGE_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (name, arm) in [
        (
            "acknowledge prepared Hitpoints 40 before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge eight Trout before Start",
            Proof::ItemId {
                id: TROUT_ID,
                count: AUTO_FIGHTER_FOOD,
            },
        ),
        (
            "acknowledge 150 Mind runes before Start",
            Proof::ItemId {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS,
            },
        ),
        (
            "acknowledge 300 Air runes before Start",
            Proof::ItemId {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES,
            },
        ),
        (
            "acknowledge Staff of fire before wielding",
            Proof::ItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(Step {
        name: "wield and acknowledge Staff of fire before hostile-field teleport",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).wear(STAFF_OF_FIRE_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "teleport to the Ardougne Guard only after mage preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(ARDOUGNE_GUARD.level, ARDOUGNE_GUARD.x, ARDOUGNE_GUARD.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ARDOUGNE_GUARD.x,
                z: ARDOUGNE_GUARD.z,
                level: ARDOUGNE_GUARD.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch native Fire Strike autocast become armed",
            Proof::Varp {
                id: AUTOCAST_MAGIC_VARP,
                min: AUTOCAST_ARMED_VALUE,
            },
        ),
        ("watch Magic XP from real Fire Strike combat", magic_xp),
        (
            "watch a Mind rune consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS - 1,
            },
        ),
        (
            "watch two Air runes consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES - 2,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "auto_fighter_mage",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: magic_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("AutoFighter"),
            script_settings_inject: Some(AUTO_FIGHTER_MAGE_INJECT),
            terminal_shot: Some("auto_fighter_mage"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn auto_fighter_range_scenario() -> Scenario {
    combat_range_scenario(
        "auto_fighter_range",
        "AutoFighter",
        ARDOUGNE_GUARD,
        6,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        AUTO_FIGHTER_RANGE_INJECT,
        false,
    )
}

/// RockCrab default melee/strength at spot 1. Ordinary bank policy Off.
/// Catalog requires native Rocks activation into Rock Crab. Banking is not
/// this cell. SolveClue stays injected off.
fn rock_crab_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "rock_crab",
        card: "RockCrab",
        tele: ROCK_CRAB_SAFE_STAND,
        radius: 2,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: ROCK_CRAB_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: ROCK_CRAB_LOOT_EMPTY,
        inject: ROCK_CRAB_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    });
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.insert(
        start,
        bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ),
    );
    scenario
}

fn rock_crab_range_scenario() -> Scenario {
    combat_range_scenario(
        "rock_crab_range",
        "RockCrab",
        ROCK_CRAB_SAFE_STAND,
        2,
        "lobster",
        LOBSTER_ID,
        ROCK_CRAB_FOOD,
        ROCK_CRAB_RANGE_INJECT,
        true,
    )
}

/// GreenDragon melee in the wilderness field. Shield 1540 is worn with the
/// native fixture operation on the safe tile before hostile-field teleport;
/// this does not qualify the frozen GearEquip branch. Catalog Start baseline
/// and cycle both require worn 1540 plus real dragon combat and exact bones 536
/// or green hide 1753. Escape/bank is not this cell.
fn green_dragon_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "green_dragon",
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear_id: Some(DRAGONFIRE_SHIELD_ID),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

fn green_dragon_special_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "green_dragon_special",
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "dragon_dagger",
        weapon_id: DRAGON_DAGGER_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear_id: Some(DRAGONFIRE_SHIELD_ID),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_SPECIAL_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    });
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            Step {
                name: "prepare and acknowledge Attack 60 for the Dragon dagger before Start",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, &format!("setstat attack {DRAGON_DAGGER_ATTACK_LEVEL}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat {
                        id: 0,
                        min: DRAGON_DAGGER_ATTACK_LEVEL,
                    },
                    budget_ticks: 200,
                },
            },
            wear_combat_item_step(
                "wield and acknowledge Dragon dagger before hostile-field teleport",
                DRAGON_DAGGER_ID,
            ),
            // The card only arms when `Special.energy() >= cost`, so the pool
            // has to already cover the dagger's 250 before Start; a low pool
            // must fail this acknowledgement instead of passing unarmed.
            bank_fletcher_watch(
                "acknowledge the worn dagger's special cost is covered before Start",
                Proof::Varp {
                    id: SPECIAL_ENERGY_VARP,
                    min: DRAGON_DAGGER_SPECIAL_COST,
                },
            ),
        ],
    );
    scenario
}

fn green_dragon_potions_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "green_dragon_potions",
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("3dose2attack", SUPER_ATTACK_3_ID, 1),
            ("3dose2strength", SUPER_STRENGTH_3_ID, 1),
        ],
        wear_id: Some(DRAGONFIRE_SHIELD_ID),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_POTIONS_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    });
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            bank_fletcher_watch(
                "confirm no seeded Super attack(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_2_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "confirm no seeded Super strength(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_STRENGTH_2_ID,
                    count: 0,
                },
            ),
        ],
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.splice(
        start + 1..start + 1,
        [
            bank_fletcher_watch(
                "watch the Super attack(3) dose leave the pack after Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_3_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "watch the Super attack(2) dose enter the pack after Start",
                Proof::ItemId {
                    id: SUPER_ATTACK_2_ID,
                    count: 1,
                },
            ),
            bank_fletcher_watch(
                "watch the native Super attack boost before further combat",
                Proof::Stat { id: 0, min: 41 },
            ),
        ],
    );
    scenario
}

/// FireGiant melee already inside the east dungeon room. Approach, barrel
/// escape and bank are unqualified. Waterfall Quest plus amulet 295 and
/// rope 954 use existing allowed preparation.
fn fire_giant_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "fire_giant",
        card: "FireGiant",
        tele: FIRE_GIANT_ROOM,
        radius: 10,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: FIRE_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        wear_id: None,
        loot_empty: FIRE_GIANT_LOOT_EMPTY,
        inject: FIRE_GIANT_INJECT,
        complete_quest: Some("Waterfall Quest"),
        thieving: 0,
        agility: 0,
    })
}

/// ArdyFighter default Guard/strength. No fabricated cakes; the script
/// must steal cake/bread/chocolate slice after Start. PeriodicBank Off.
fn ardy_fighter_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "ardy_fighter",
        card: "ArdyFighter",
        tele: ARDOUGNE_GUARD,
        radius: 12,
        food_alias: "cake",
        food_id: CAKE_ID,
        food_count: 0,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear_id: None,
        loot_empty: ARDY_FIGHTER_LOOT_EMPTY,
        inject: ARDY_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 5,
        agility: 0,
    })
}

/// `banking=Auto` on AutoFighter: BankRun walks to the nearest bank from the
/// anchor, deposits everything its keep-list does not hold and restocks food.
/// `loot` is the six verifiable Guard drops (the card's default gem+clue list
/// is unreachable: Guards drop no gems and the card keeps clue items), and
/// `bankAtLootSlots=1` so the first of those drops is what ends the trip.
const AUTO_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&[
            "iron ore",
            "steel arrow",
            "body talisman",
            "blood rune",
            "chaos rune",
            "nature rune",
        ]),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];

/// HillGiant's always-on trip end, reached on the first loot slot so the cell
/// does not need fourteen giant drops. `meleeStyle`/`buryBones` as the core.
const HILL_GIANT_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "lootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
];

/// ArdyFighter's `bankStrategy=Loot count` PeriodicBank after it has looted a
/// Guard drop. `foodTarget=1` keeps the stall restock short so the cell has
/// room for the loot the bank trip deposits.
const ARDY_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];

/// Shared bank-cell fixture: the same safe-tile preparation and acknowledgement
/// order as `combat_core_scenario`, plus the acknowledged bank stock the trip
/// has to draw from, a wielded (never carried) weapon so the card's own deposit
/// cannot stash it, and the bank/return watch chain after Start.
#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    thieving: i32,
    bank_alias: &'static str,
    bank_food_count: i32,
    watches: &[(&'static str, Proof)],
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare melee stats, trip stock and bank stock on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {food_alias} {food_count}"));
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                if bank_food_count > 0 {
                    cheat(c, &format!("givebank {bank_alias} {bank_food_count}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Attack 40",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge prepared Strength 40",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge prepared Hitpoints 40",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if food_count > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge the trip's carried food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded kill loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the weapon carried onto the safe tile",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    steps.push(wear_combat_item_step(
        "wield and acknowledge the weapon before the hostile-field teleport",
        weapon_id,
    ));
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
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
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch Strength XP from the selected melee style after Start",
        xp,
    ));
    for (step_name, arm) in watches {
        steps.push(bank_fletcher_watch(step_name, *arm));
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
            start_script: Some(card),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// `banking=Auto` on the Guard anchor: the injected `loot` list is the six
/// verifiable Guard drops and `bankAtLootSlots=1` makes the first of them the
/// trip's loot slot. East Ardougne is the nearest bank, and the BankRun has to
/// deposit that loot, restock trout to its declared 10, close, walk back to
/// the anchor and fight again. Nothing in the class is prepared: it can only
/// enter the pack as a Guard drop, and a run that never loots one deposits
/// nothing to qualify.
fn auto_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "auto_fighter_bank",
        "AutoFighter",
        ARDOUGNE_GUARD,
        8,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        COMBAT_SCIMITAR_ID,
        &[],
        &GUARD_DROP_IDS,
        AUTO_FIGHTER_BANK_INJECT,
        0,
        "trout",
        20,
        &[
            (
                "watch a Guard drop enter a fresh Ardougne East bank",
                Proof::BankItemIdAny {
                    ids: &GUARD_DROP_IDS,
                    count: 1,
                },
            ),
            (
                "watch the BankRun restock Trout to its declared ten",
                Proof::ItemId {
                    id: TROUT_ID,
                    count: AUTO_FIGHTER_BANK_RESTOCK,
                },
            ),
            ("watch the BankRun close the booth", Proof::BankClosed),
            (
                "watch return to the Guard anchor after banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}

/// MossGiant's own trip end (no food left) reached after a looted Big bones,
/// so the Ardougne West booth deposit and lobster restock are the source's
/// transitions, not a seeded stock move.
fn moss_giant_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "moss_giant_bank",
        "MossGiant",
        MOSS_GIANT_SAFESPOT,
        10,
        "lobster",
        LOBSTER_ID,
        MOSS_GIANT_BANK_FOOD,
        COMBAT_SCIMITAR_ID,
        &[],
        MOSS_GIANT_LOOT_EMPTY,
        MOSS_GIANT_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch the trip's Big bones enter a fresh Ardougne West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the restock of Lobster to the card's declared line",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: MOSS_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch MossGiant close its bank", Proof::BankClosed),
            (
                "watch return to the moss-giant safespot after banking",
                Proof::ArrivedNear {
                    x: MOSS_GIANT_SAFESPOT.x,
                    z: MOSS_GIANT_SAFESPOT.z,
                    level: MOSS_GIANT_SAFESPOT.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}

/// HillGiant's always-on trip end (`lootSlots=1`): one looted Giant drop ends
/// the trip, Varrock West banks it and withdraws trout back, then the pit
/// fight resumes.
fn hill_giant_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "hill_giant_bank",
        "HillGiant",
        HILL_GIANT_PIT,
        16,
        "trout",
        TROUT_ID,
        HILL_GIANT_FOOD,
        COMBAT_SCIMITAR_ID,
        &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        HILL_GIANT_LOOT_EMPTY,
        HILL_GIANT_BANK_INJECT,
        0,
        "trout",
        12,
        &[
            (
                "watch the trip's Big bones enter a fresh Varrock West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the restock of Trout to the card's declared twelve",
                Proof::ItemId {
                    id: TROUT_ID,
                    count: HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch HillGiant close its bank", Proof::BankClosed),
            (
                "watch return to the giant pit after banking",
                Proof::ArrivedNear {
                    x: HILL_GIANT_PIT.x,
                    z: HILL_GIANT_PIT.z,
                    level: HILL_GIANT_PIT.level,
                    radius: 16,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}

/// ChaosDruidKiller's own `prepare-trip` end: the pack carries less than
/// `foodWithdraw`, so the card climbs out, deposits the pack at the Edgeville
/// booth, withdraws its twelve lobster, closes and returns through the trapdoor
/// to the field, where the kills, loot and further work have to follow.
fn chaos_druid_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "chaos_druid_bank",
        "ChaosDruidKiller",
        CHAOS_DRUID_FIELD,
        14,
        "lobster",
        LOBSTER_ID,
        CHAOS_DRUID_BANK_FOOD,
        COMBAT_SCIMITAR_ID,
        &[],
        CHAOS_DRUID_LOOT_EMPTY,
        CHAOS_DRUID_INJECT,
        0,
        "lobster",
        12,
        &[
            (
                "watch the prepare-trip restock of exactly twelve Lobster",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: CHAOS_DRUID_FOOD,
                },
            ),
            ("watch the Edgeville bank close", Proof::BankClosed),
            (
                "watch return through the trapdoor into the druid field",
                Proof::ArrivedNear {
                    x: CHAOS_DRUID_FIELD.x,
                    z: CHAOS_DRUID_FIELD.z,
                    level: CHAOS_DRUID_FIELD.level,
                    radius: 14,
                },
            ),
            (
                "watch fresh Strength XP inside the field after the return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}

/// ArdyFighter's `bankStrategy=Loot count` trip: after a Guard drop lands in
/// the pack the PeriodicBank walks to the East Ardougne booth, deposits the
/// card's own loot list and returns to the market anchor for further work.
fn ardy_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "ardy_fighter_bank",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        0,
        COMBAT_SCIMITAR_ID,
        &[],
        ARDY_FIGHTER_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_INJECT,
        5,
        "",
        0,
        &[
            ("watch the periodic bank close", Proof::BankClosed),
            (
                "watch return to the market anchor after banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
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
                "alcher_defaults",
                "alcher_custom",
                "alcher_custom_alias",
                "alcher_custom_name",
                "alcher_ordered",
                "alcher_large_batch",
                "bank_fletcher",
                "bank_fletcher_shafts",
                "bank_fletcher_headless",
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
                "wildy_agility",
                "brimhaven_agility",
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
                "rune_crafter",
                "rune_crafter_earth",
                "mule_crafter",
                "ardy_cakes",
                "ardy_cakes_fight",
                "ardy_thiever",
                "ardy_thiever_fight",
                "ardy_thiever_knight",
                "gnome_chop",
                "gnome_fletch_short",
                "gnome_fletch_long",
                "coal_trucks",
                "cook_bot",
                "cook_bot_lobster",
                "smelter_bot",
                "smelter_bot_steel",
                "flax_spinner",
                "flax_aio",
                "flax_aio_pick",
                "flax_aio_spin",
                "herblore_secondaries",
                "herblore_secondaries_newt",
                "chaos_druid",
                "chaos_druid_tower",
                "chaos_druid_yanille",
                "moss_giant",
                "hill_giant",
                "auto_fighter",
                "auto_fighter_mage",
                "auto_fighter_range",
                "rock_crab",
                "rock_crab_range",
                "green_dragon",
                "green_dragon_special",
                "green_dragon_potions",
                "fire_giant",
                "ardy_fighter",
                "auto_fighter_bank",
                "moss_giant_bank",
                "hill_giant_bank",
                "chaos_druid_bank",
                "ardy_fighter_bank",
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
    fn remaining_production_options_are_registered_with_ordered_exact_proofs() {
        let defaults = get("alcher_defaults").expect("default Alcher scenario registered");
        assert_eq!(defaults.settings.start_script, Some("Alcher"));
        assert_eq!(defaults.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(defaults.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("items"), Some(&Value::Array(vec![])));
        assert_eq!(inject.get("alchs"), Some(&Value::from(1.0)));
        let start = defaults
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = defaults.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::BankItemId { id: 855, count: 1 }));
        assert!(seed.contains(&Proof::BankItemIdAtMost { id: 856, count: 0 }));
        assert!(seed.contains(&Proof::ItemAtMost {
            name: "Rune chainbody",
            count: 0,
        }));
        assert_eq!(defaults.steps[start - 1].wait.arm, Proof::BankClosed);
        assert_eq!(
            defaults.steps[start + 1..]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>(),
            vec![
                Proof::ItemId { id: 856, count: 1 },
                Proof::StatXpGain { id: 6, min: 65 },
                Proof::ItemIdAtMost { id: 856, count: 0 },
                Proof::ItemIdAtMost { id: 561, count: 0 },
                Proof::ItemId {
                    id: 995,
                    count: 768
                },
                Proof::ItemAtMost {
                    name: "Rune chainbody",
                    count: 0,
                },
            ]
        );

        for (name, material, product, first_product, first_count) in [
            ("bank_fletcher_shafts", "Logs", "Arrow shafts", 52, 405),
            ("bank_fletcher_headless", "Logs", "Headless arrows", 53, 30),
        ] {
            let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
            assert_eq!(scenario.settings.start_script, Some("BankFletcher"));
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
            let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
            assert_eq!(
                inject.get("material"),
                Some(&Value::String(material.into()))
            );
            assert_eq!(inject.get("product"), Some(&Value::String(product.into())));
            assert!(!inject.contains_key("mode"));
            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap();
            assert_eq!(scenario.steps[start - 1].wait.arm, Proof::BankClosed);
            let watch = scenario.steps[start + 1..]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>();
            assert_eq!(watch[0], Proof::StatXpGain { id: 9, min: 1 });
            assert!(watch.contains(&Proof::ItemId {
                id: first_product,
                count: first_count,
            }));
            let closed = watch
                .iter()
                .position(|arm| *arm == Proof::BankClosed)
                .unwrap();
            assert!(matches!(
                watch[closed + 1],
                Proof::FreshStatXpGain { id: 9, min: 1 }
            ));
            assert_eq!(
                scenario.proof,
                Proof::ItemId {
                    id: first_product,
                    count: 1
                }
            );
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn bank_cells_register_their_cards_injects_and_watch_chain() {
        for (name, card) in [
            ("auto_fighter_bank", "AutoFighter"),
            ("moss_giant_bank", "MossGiant"),
            ("hill_giant_bank", "HillGiant"),
            ("chaos_druid_bank", "ChaosDruidKiller"),
            ("ardy_fighter_bank", "ArdyFighter"),
        ] {
            let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
            assert_eq!(scenario.settings.start_script, Some(card));
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
            assert_eq!(scenario.settings.terminal_shot, Some(name));
            assert!(scenario.settings.full_rate && scenario.seed.mainland);
            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap_or_else(|| panic!("{name} starts the card"));
            assert!(
                scenario.steps[..start].iter().any(|step| step.wait.arm
                    == Proof::EquipmentId {
                        id: COMBAT_SCIMITAR_ID
                    }),
                "{name} wields its weapon before the hostile-field teleport"
            );
            let watch = scenario.steps[start + 1..]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>();
            assert!(
                watch.contains(&Proof::BankClosed),
                "{name} watches its own bank close"
            );
            assert!(
                watch.iter().any(|arm| matches!(
                    arm,
                    Proof::FreshStatXpGain {
                        id: STRENGTH_STAT,
                        min: 1
                    }
                )),
                "{name} needs fresh work after the bank return"
            );
        }
        let auto = settings_inject_map(
            get("auto_fighter_bank")
                .unwrap()
                .settings
                .script_settings_inject,
        )
        .unwrap();
        assert_eq!(auto.get("banking"), Some(&Value::String("Auto".into())));
        assert_eq!(auto.get("bankAtLootSlots"), Some(&Value::from(1.0)));
        // The trip loot is the injected Guard list itself: no deposit-class
        // item is prepared, so the class can only arrive as a kill drop.
        assert_eq!(
            auto.get("loot"),
            Some(&Value::Array(
                [
                    "iron ore",
                    "steel arrow",
                    "body talisman",
                    "blood rune",
                    "chaos rune",
                    "nature rune",
                ]
                .into_iter()
                .map(|name| Value::String(name.into()))
                .collect()
            ))
        );
        let auto_scenario = get("auto_fighter_bank").unwrap();
        let auto_start = auto_scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        for step in &auto_scenario.steps[..auto_start] {
            assert!(
                !matches!(step.wait.arm, Proof::ItemId { id, .. } if GUARD_DROP_IDS.contains(&id)),
                "auto_fighter_bank must not prepare a deposit-class item before Start"
            );
        }
        for &id in &GUARD_DROP_IDS {
            assert!(
                auto_scenario.steps[..auto_start].iter().any(|step| {
                    matches!(step.wait.arm, Proof::ItemIdAtMost { id: got, count: 0 } if got == id)
                }),
                "auto_fighter_bank must confirm empty deposit-class id {id} before Start"
            );
        }
        let hill = settings_inject_map(
            get("hill_giant_bank")
                .unwrap()
                .settings
                .script_settings_inject,
        )
        .unwrap();
        assert_eq!(hill.get("lootSlots"), Some(&Value::from(1.0)));
        assert_eq!(hill.get("buryBones"), Some(&Value::Bool(false)));
        let chaos = settings_inject_map(
            get("chaos_druid_bank")
                .unwrap()
                .settings
                .script_settings_inject,
        )
        .unwrap();
        assert_eq!(
            chaos.get("location"),
            Some(&Value::String("Edgeville Dungeon".into()))
        );
        let ardy = settings_inject_map(
            get("ardy_fighter_bank")
                .unwrap()
                .settings
                .script_settings_inject,
        )
        .unwrap();
        assert_eq!(
            ardy.get("bankStrategy"),
            Some(&Value::String("Loot count".into()))
        );
        assert_eq!(ardy.get("bankEveryItems"), Some(&Value::from(1.0)));
        assert_eq!(ardy.get("target"), Some(&Value::String("Guard".into())));
    }

    #[test]
    fn hostile_fields_follow_safe_pretele_precondition_acknowledgements() {
        for name in [
            "chaos_druid",
            "moss_giant",
            "hill_giant",
            "auto_fighter",
            "rock_crab",
            "green_dragon",
            "fire_giant",
            "ardy_fighter",
        ] {
            let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap();
            let arrival = scenario.steps[..start]
                .iter()
                .position(|step| matches!(step.wait.arm, Proof::ArrivedNear { .. }))
                .unwrap_or_else(|| panic!("{name} field arrival"));
            for proof in [
                Proof::Stat {
                    id: 0,
                    min: COMBAT_ATTACK_LEVEL,
                },
                Proof::Stat {
                    id: STRENGTH_STAT,
                    min: COMBAT_ATTACK_LEVEL,
                },
                Proof::Stat {
                    id: 3,
                    min: COMBAT_ATTACK_LEVEL,
                },
            ] {
                let ack = scenario.steps[..start]
                    .iter()
                    .position(|step| step.wait.arm == proof)
                    .unwrap_or_else(|| panic!("{name} missing {proof:?}"));
                assert!(
                    ack < arrival,
                    "{name} must acknowledge {proof:?} on the safe tile"
                );
            }
            for step in &scenario.steps[arrival + 1..start] {
                assert!(
                    !matches!(
                        step.wait.arm,
                        Proof::Stat { .. }
                            | Proof::Item { .. }
                            | Proof::ItemId { .. }
                            | Proof::ItemIdAtMost { .. }
                    ),
                    "{name} leaves a precondition acknowledgement after the hostile-field tele: {}",
                    step.name
                );
            }
        }

        let coal = get("coal_trucks").expect("coal_trucks");
        let start = coal
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let arrival = coal.steps[..start]
            .iter()
            .position(|step| matches!(step.wait.arm, Proof::ArrivedNear { .. }))
            .expect("coal-mine arrival");
        for proof in [
            Proof::Stat { id: 0, min: 48 },
            Proof::Stat {
                id: STRENGTH_STAT,
                min: 48,
            },
            Proof::Stat { id: 1, min: 48 },
            Proof::Stat { id: 3, min: 48 },
            Proof::Stat {
                id: MINING_STAT,
                min: 60,
            },
            Proof::ItemId {
                id: RUNE_PICKAXE_ID,
                count: 1,
            },
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ] {
            let ack = coal.steps[..start]
                .iter()
                .position(|step| step.wait.arm == proof)
                .unwrap_or_else(|| panic!("coal_trucks missing {proof:?}"));
            assert!(
                ack < arrival,
                "coal_trucks must acknowledge {proof:?} before the bat mine"
            );
        }
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
    fn wildy_agility_registers_the_full_ordered_course_on_the_observed_player_plane() {
        let wildy = get("wildy_agility").expect("wildy_agility");
        assert_eq!(wildy.settings.start_script, Some("WildyAgility"));
        assert_eq!(wildy.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(wildy.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("acquireFoodAtStart"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("minFood").and_then(Value::as_f64), Some(0.0));

        let start = wildy
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("StartScript");
        let seed = wildy.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(wildy.steps[..start]
            .iter()
            .any(|step| step.name.contains("Agility 52")));
        assert!(seed.contains(&Proof::Stat {
            id: HITPOINTS_STAT,
            min: 40,
        }));
        assert!(seed.contains(&Proof::ItemId { id: 379, count: 5 }));
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2998,
            z: 3916,
            level: 0,
            radius: 2,
        }));

        let watch = wildy.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 15,
                },
                Proof::ArrivedNear {
                    x: 2998,
                    z: 3934,
                    level: 0,
                    radius: 2,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 27,
                },
                Proof::ArrivedNear {
                    x: 3004,
                    z: 3947,
                    level: 0,
                    radius: 3,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 47,
                },
                Proof::ArrivedNear {
                    x: 3005,
                    z: 3958,
                    level: 0,
                    radius: 3,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 67,
                },
                Proof::ArrivedNear {
                    x: 2996,
                    z: 3960,
                    level: 0,
                    radius: 3,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 87,
                },
                Proof::ArrivedNear {
                    x: 2994,
                    z: 3945,
                    level: 0,
                    radius: 3,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 586,
                },
                Proof::ArrivedNear {
                    x: 2994,
                    z: 3933,
                    level: 0,
                    radius: 3,
                },
                Proof::ArrivedNear {
                    x: 3004,
                    z: 3947,
                    level: 0,
                    radius: 3,
                },
            ]
        );
        assert_eq!(
            wildy.proof,
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 598,
            }
        );
        assert!(names().contains(&"wildy_agility"));
    }

    #[test]
    fn brimhaven_agility_registers_fee_tags_ticket_and_subsequent_xp() {
        let brim = get("brimhaven_agility").expect("brimhaven_agility");
        assert_eq!(brim.settings.start_script, Some("BrimhavenAgility"));
        assert_eq!(brim.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(brim.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("stealRestock"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("bankAtTickets").and_then(Value::as_f64),
            Some(1000.0)
        );

        let start = brim
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("StartScript");
        let seed = brim.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(brim.steps[..start].iter().any(|step| step.name
            == "seed Agility 52, 1000 Coins and ten Lobsters, then tele to the arena entrance"));
        assert!(seed.contains(&Proof::ChatClosed));
        assert!(seed.contains(&Proof::ItemId {
            id: 995,
            count: 1000,
        }));
        assert!(seed.contains(&Proof::ItemId { id: 379, count: 10 }));
        assert!(seed.contains(&Proof::ItemIdAtMost { id: 2996, count: 0 }));
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2809,
            z: 3194,
            level: 0,
            radius: 2,
        }));
        assert!(!seed
            .iter()
            .any(|proof| matches!(proof, Proof::ItemId { id: 2996, .. })));

        let watch = brim.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch,
            vec![
                Proof::ItemIdAtMost {
                    id: 995,
                    count: 800,
                },
                Proof::Varp { id: 309, min: 2 },
                Proof::ArrivedNear {
                    x: 2805,
                    z: 9590,
                    level: 3,
                    radius: 2,
                },
                Proof::StatXpGain {
                    id: AGILITY_STAT,
                    min: 1,
                },
                Proof::Chat {
                    needle: "tag the next",
                },
                Proof::Varp { id: 309, min: 15 },
                Proof::ItemIdAtMost { id: 2996, count: 0 },
                Proof::ItemId { id: 2996, count: 1 },
                Proof::FreshStatXpGain {
                    id: AGILITY_STAT,
                    min: 1,
                },
            ]
        );
        assert_eq!(
            brim.proof,
            Proof::FreshStatXpGain {
                id: AGILITY_STAT,
                min: 1,
            }
        );
        assert!(names().contains(&"brimhaven_agility"));
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
            z: 3368,
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
    fn rune_crafter_cases_register_altar_conversion_and_bank_cycles() {
        let air = get("rune_crafter").expect("rune_crafter");
        assert_eq!(air.settings.start_script, Some("RuneCrafter"));
        assert_eq!(air.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(air.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("rune"), Some(&Value::String("Air runes".into())));
        assert_eq!(inject.get("mode"), Some(&Value::String("Solo".into())));
        let start = air
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert_eq!(air.steps[start - 1].wait.arm, Proof::BankClosed);
        let seed = air.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3013,
            z: 3355,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: RUNECRAFT_STAT,
            min: 1,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: RUNE_ESSENCE_ID,
            count: RUNE_ESSENCE_SEED,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: AIR_TALISMAN_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: AIR_RUNE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemIdAtMost {
            id: AIR_RUNE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        }));
        let watch = air.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let ruins = Proof::ArrivedNear {
            x: 2988,
            z: 3294,
            level: 0,
            radius: 4,
        };
        let crafted = Proof::ItemId {
            id: AIR_RUNE_ID,
            count: 1,
        };
        let pack_empty_runes = Proof::ItemIdAtMost {
            id: AIR_RUNE_ID,
            count: 0,
        };
        assert_eq!(
            watch,
            vec![
                Proof::ItemId {
                    id: RUNE_ESSENCE_ID,
                    count: 1,
                },
                ruins,
                Proof::StatXpGain {
                    id: RUNECRAFT_STAT,
                    min: 1,
                },
                crafted,
                Proof::ItemIdAtMost {
                    id: RUNE_ESSENCE_ID,
                    count: 0,
                },
                ruins,
                Proof::BankItemId {
                    id: AIR_RUNE_ID,
                    count: 1,
                },
                pack_empty_runes,
                Proof::ItemId {
                    id: RUNE_ESSENCE_ID,
                    count: 1,
                },
                Proof::BankClosed,
                ruins,
                crafted,
            ]
        );
        let first_xp = watch
            .iter()
            .position(|arm| {
                *arm == Proof::StatXpGain {
                    id: RUNECRAFT_STAT,
                    min: 1,
                }
            })
            .unwrap();
        let first_rune = watch.iter().position(|arm| *arm == crafted).unwrap();
        let bank_runes = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: AIR_RUNE_ID,
                    count: 1,
                }
            })
            .unwrap();
        let pack_empty = watch
            .iter()
            .position(|arm| *arm == pack_empty_runes)
            .unwrap();
        let further_rune = watch.iter().rposition(|arm| *arm == crafted).unwrap();
        assert!(watch[1] == ruins);
        assert!(first_xp < first_rune);
        assert!(first_rune < bank_runes);
        assert!(bank_runes < pack_empty);
        assert!(pack_empty < further_rune);
        assert_ne!(first_rune, further_rune);
        assert_eq!(air.proof, crafted);

        let earth = get("rune_crafter_earth").expect("rune_crafter_earth");
        let inject = settings_inject_map(earth.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("rune"),
            Some(&Value::String("Earth runes".into()))
        );
        assert_eq!(inject.get("mode"), Some(&Value::String("Solo".into())));
        let earth_start = earth
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let earth_seed = earth.steps[..earth_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(earth_seed.contains(&Proof::ArrivedNear {
            x: 3253,
            z: 3420,
            level: 0,
            radius: 6,
        }));
        assert!(earth_seed.contains(&Proof::Stat {
            id: RUNECRAFT_STAT,
            min: 9,
        }));
        let earth_watch = earth.steps[earth_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let earth_ruins = Proof::ArrivedNear {
            x: 3303,
            z: 3477,
            level: 0,
            radius: 4,
        };
        let earth_rune = Proof::ItemId {
            id: EARTH_RUNE_ID,
            count: 1,
        };
        assert_eq!(earth_watch[1], earth_ruins);
        assert_eq!(
            earth_watch[2],
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            }
        );
        assert_eq!(earth_watch[3], earth_rune);
        assert!(earth_watch.contains(&Proof::BankItemId {
            id: EARTH_RUNE_ID,
            count: 1,
        }));
        assert!(earth_watch.contains(&Proof::ItemIdAtMost {
            id: EARTH_RUNE_ID,
            count: 0,
        }));
        assert!(!earth_watch.contains(&crafted));
        assert_eq!(earth.proof, earth_rune);

        let mule = get("mule_crafter").expect("mule_crafter");
        assert_eq!(mule.settings.start_script, Some("MuleCrafter"));
        let inject = settings_inject_map(mule.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("rune"), Some(&Value::String("Air rune".into())));
        assert_eq!(inject.get("mode"), Some(&Value::String("Crafter".into())));
        assert_eq!(inject.get("partner"), Some(&Value::String("".into())));
        assert_eq!(inject.get("bankFill"), Some(&Value::Bool(true)));
        let mule_start = mule
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let mule_watch = mule.steps[mule_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let mule_ruins = Proof::ArrivedNear {
            x: 2983,
            z: 3288,
            level: 0,
            radius: 4,
        };
        assert_eq!(mule_watch[1], mule_ruins);
        assert_eq!(
            mule_watch[2],
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            }
        );
        assert_eq!(mule_watch[3], crafted);
        assert_ne!(mule_watch[1], ruins);
        assert_eq!(mule.proof, crafted);

        for name in ["rune_crafter", "rune_crafter_earth", "mule_crafter"] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn ardy_thieving_cases_register_stall_guard_knight_and_bank_cycles() {
        let cakes = get("ardy_cakes").expect("ardy_cakes");
        assert_eq!(cakes.settings.start_script, Some("ArdyCakes"));
        assert_eq!(cakes.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(cakes.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("guardResponse"),
            Some(&Value::String("Flee".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert!(inject.get("bankStrategy").is_none());
        let start = cakes
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = cakes.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2668,
            z: 3312,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 5,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: HITPOINTS_STAT,
            min: 40,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CAKE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BREAD_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CHOCOLATE_SLICE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CHOCOLATE_CAKE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: KNIFE_ID,
            count: 22,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: KNIFE_ID,
            count: 22,
        }));
        let watch = cakes.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let cake = Proof::ItemId {
            id: CAKE_ID,
            count: 1,
        };
        let pack_empty_cake = Proof::ItemIdAtMost {
            id: CAKE_ID,
            count: 0,
        };
        let stand = Proof::ArrivedNear {
            x: 2668,
            z: 3312,
            level: 0,
            radius: 6,
        };
        let bank = Proof::ArrivedNear {
            x: ARDY_BANK.x,
            z: ARDY_BANK.z,
            level: ARDY_BANK.level,
            radius: 6,
        };
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: THIEVING_STAT,
                    min: 1
                },
                cake,
                bank,
                Proof::BankItemId {
                    id: CAKE_ID,
                    count: 1
                },
                pack_empty_cake,
                stand,
                Proof::BankClosed,
                cake,
            ]
        );
        let first_cake = watch.iter().position(|arm| *arm == cake).unwrap();
        let bank_cake = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: CAKE_ID,
                    count: 1,
                }
            })
            .unwrap();
        let pack_empty = watch
            .iter()
            .position(|arm| *arm == pack_empty_cake)
            .unwrap();
        let further_cake = watch.iter().rposition(|arm| *arm == cake).unwrap();
        assert!(first_cake < bank_cake);
        assert!(bank_cake < pack_empty);
        assert!(pack_empty < further_cake);
        assert_ne!(first_cake, further_cake);
        assert_eq!(cakes.proof, cake);
        assert_eq!(
            ARDY_BANK,
            WorldTile {
                x: 2655,
                z: 3286,
                level: 0
            }
        );

        let guard = get("ardy_thiever").expect("ardy_thiever");
        assert_eq!(guard.settings.start_script, Some("ArdyThiever"));
        let inject = settings_inject_map(guard.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("thieveTarget"),
            Some(&Value::String("Guard".into()))
        );
        assert_eq!(
            inject.get("guardResponse"),
            Some(&Value::String("Flee".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("bankAtLootSlots"), Some(&Value::from(1.0)));
        assert_eq!(inject.get("foodTarget"), Some(&Value::from(1.0)));
        assert_eq!(inject.get("restockAtFood"), Some(&Value::from(0.0)));
        assert!(inject.get("bankStrategy").is_none());
        let guard_start = guard
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let guard_seed = guard.steps[..guard_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(guard_seed.contains(&Proof::ArrivedNear {
            x: 2661,
            z: 3306,
            level: 0,
            radius: 8,
        }));
        assert!(guard_seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 40,
        }));
        assert!(guard_seed.contains(&Proof::ItemIdAtMost {
            id: COINS_ID,
            count: 0,
        }));
        let guard_watch = guard.steps[guard_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let coins = Proof::ItemId {
            id: COINS_ID,
            count: 1,
        };
        let pack_empty_coins = Proof::ItemIdAtMost {
            id: COINS_ID,
            count: 0,
        };
        let market = Proof::ArrivedNear {
            x: 2661,
            z: 3306,
            level: 0,
            radius: 6,
        };
        assert_eq!(
            guard_watch,
            vec![
                Proof::StatXpGain {
                    id: THIEVING_STAT,
                    min: 1
                },
                coins,
                Proof::BankItemId {
                    id: COINS_ID,
                    count: 1
                },
                pack_empty_coins,
                market,
                Proof::BankClosed,
                coins,
            ]
        );
        let first_coins = guard_watch.iter().position(|arm| *arm == coins).unwrap();
        let further_coins = guard_watch.iter().rposition(|arm| *arm == coins).unwrap();
        assert_ne!(first_coins, further_coins);
        assert_eq!(guard.proof, coins);

        let knight = get("ardy_thiever_knight").expect("ardy_thiever_knight");
        assert_eq!(knight.settings.start_script, Some("ArdyThiever"));
        let inject = settings_inject_map(knight.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("thieveTarget"),
            Some(&Value::String("Knight of Ardougne".into()))
        );
        assert_eq!(inject.get("bankAtLootSlots"), Some(&Value::from(1.0)));
        let knight_start = knight
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let knight_seed = knight.steps[..knight_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(knight_seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 55,
        }));
        assert!(!knight_seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 40,
        }));
        let knight_watch = knight.steps[knight_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(knight_watch, guard_watch);
        assert_eq!(knight.proof, coins);

        for name in ["ardy_cakes", "ardy_thiever", "ardy_thiever_knight"] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn alternate_camp_and_fight_option_cases_register() {
        let strength = Proof::StatXpGain {
            id: STRENGTH_STAT,
            min: 1,
        };
        let cakes_fight = get("ardy_cakes_fight").expect("ardy_cakes_fight");
        assert_eq!(cakes_fight.settings.start_script, Some("ArdyCakes"));
        assert_eq!(cakes_fight.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(cakes_fight.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("guardResponse"),
            Some(&Value::String("Fight".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        let start = cakes_fight
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = cakes_fight.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2668,
            z: 3312,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 5,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: 0,
            min: COMBAT_ATTACK_LEVEL,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: STRENGTH_STAT,
            min: COMBAT_ATTACK_LEVEL,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: KNIFE_ID,
            count: 22,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: COMBAT_SCIMITAR_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::EquipmentId {
            id: COMBAT_SCIMITAR_ID,
        }));
        let watch = cakes_fight.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: THIEVING_STAT,
                    min: 1
                },
                Proof::ItemId {
                    id: CAKE_ID,
                    count: 1
                },
                strength,
            ]
        );
        assert_eq!(cakes_fight.proof, strength);
        assert!(!watch
            .iter()
            .any(|arm| matches!(arm, Proof::BankItemId { .. })));

        let thiever_fight = get("ardy_thiever_fight").expect("ardy_thiever_fight");
        assert_eq!(thiever_fight.settings.start_script, Some("ArdyThiever"));
        let inject = settings_inject_map(thiever_fight.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("guardResponse"),
            Some(&Value::String("Fight".into()))
        );
        assert_eq!(
            inject.get("thieveTarget"),
            Some(&Value::String("Guard".into()))
        );
        let start = thiever_fight
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = thiever_fight.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 40,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: 0,
            min: COMBAT_ATTACK_LEVEL,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: COMBAT_SCIMITAR_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::EquipmentId {
            id: COMBAT_SCIMITAR_ID,
        }));
        let watch = thiever_fight.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let coins = Proof::ItemId {
            id: COINS_ID,
            count: 1,
        };
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: THIEVING_STAT,
                    min: 1
                },
                coins,
                strength,
                Proof::BankItemId {
                    id: COINS_ID,
                    count: 1
                },
                Proof::ItemIdAtMost {
                    id: COINS_ID,
                    count: 0
                },
                Proof::ArrivedNear {
                    x: 2661,
                    z: 3306,
                    level: 0,
                    radius: 6,
                },
                Proof::BankClosed,
                coins,
            ]
        );
        assert_eq!(thiever_fight.proof, coins);

        let tower = get("chaos_druid_tower").expect("chaos_druid_tower");
        assert_eq!(tower.settings.start_script, Some("ChaosDruidKiller"));
        let inject = settings_inject_map(tower.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Chaos Druid Tower".into()))
        );
        assert_eq!(
            inject.get("combatStyleIndex"),
            Some(&Value::String("1".into()))
        );
        let start = tower
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = tower.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2562,
            z: 3356,
            level: 0,
            radius: 4,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 46,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: CHAOS_DRUID_FOOD,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: UNIDENTIFIED_GUAM_ID,
            count: 0,
        }));
        assert_eq!(tower.steps[start + 1].wait.arm, strength);
        assert_eq!(tower.proof, strength);

        let yanille = get("chaos_druid_yanille").expect("chaos_druid_yanille");
        assert_eq!(yanille.settings.start_script, Some("ChaosDruidKiller"));
        let inject = settings_inject_map(yanille.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Yanille Dungeon".into()))
        );
        let start = yanille
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = yanille.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2580,
            z: 9501,
            level: 0,
            radius: 8,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: AGILITY_STAT,
            min: 40,
        }));
        assert!(!seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 46,
        }));
        assert_eq!(yanille.steps[start + 1].wait.arm, strength);
        assert_eq!(yanille.proof, strength);

        for name in [
            "ardy_cakes_fight",
            "ardy_thiever_fight",
            "chaos_druid_tower",
            "chaos_druid_yanille",
        ] {
            assert!(names().contains(&name));
            let scenario = get(name).unwrap();
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
        }
    }

    #[test]
    fn resource_world_cases_register_gnome_log_bank_fletch_and_coal_truck() {
        let chop = get("gnome_chop").expect("gnome_chop");
        assert_eq!(chop.settings.start_script, Some("GnomeMagicChopper"));
        assert_eq!(chop.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(chop.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(false)));
        let start = chop
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = chop.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2433,
            z: 3409,
            level: 0,
            radius: 1,
        }));
        assert!(seed.contains(&Proof::LocActionNear {
            id: 1306,
            x: 2432,
            z: 3410,
            level: 0,
            radius: 0,
            action: "Chop down",
            present: true,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: WOODCUTTING_STAT,
            min: 75,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: RUNE_AXE_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: RUNE_AXE_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: KNIFE_ID,
            count: 26,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: KNIFE_ID,
            count: 26,
        }));
        assert!(!seed.contains(&Proof::ItemId { id: 1353, count: 1 }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: MAGIC_LOGS_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: UNSTRUNG_MAGIC_SHORTBOW_ID,
            count: 0,
        }));
        let watch = chop.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let logs = Proof::ItemId {
            id: MAGIC_LOGS_ID,
            count: 1,
        };
        let pack_empty_logs = Proof::ItemIdAtMost {
            id: MAGIC_LOGS_ID,
            count: 0,
        };
        let upstairs = Proof::ArrivedNear {
            x: GNOME_BANK_STAND.x,
            z: GNOME_BANK_STAND.z,
            level: GNOME_BANK_STAND.level,
            radius: 8,
        };
        let ground = Proof::ArrivedNear {
            x: GNOME_BANK_STAIR_SOUTH.x,
            z: GNOME_BANK_STAIR_SOUTH.z,
            level: GNOME_BANK_STAIR_SOUTH.level,
            radius: 30,
        };
        assert_eq!(
            watch,
            vec![
                Proof::StatXpGain {
                    id: WOODCUTTING_STAT,
                    min: 1
                },
                logs,
                upstairs,
                Proof::BankItemId {
                    id: MAGIC_LOGS_ID,
                    count: 1
                },
                pack_empty_logs,
                ground,
                Proof::BankClosed,
                logs,
            ]
        );
        let first_logs = watch.iter().position(|arm| *arm == logs).unwrap();
        let bank_logs = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: MAGIC_LOGS_ID,
                    count: 1,
                }
            })
            .unwrap();
        let pack_empty = watch
            .iter()
            .position(|arm| *arm == pack_empty_logs)
            .unwrap();
        let further_logs = watch.iter().rposition(|arm| *arm == logs).unwrap();
        assert!(first_logs < bank_logs);
        assert!(bank_logs < pack_empty);
        assert!(pack_empty < further_logs);
        assert_ne!(first_logs, further_logs);
        assert_eq!(chop.proof, logs);
        assert_eq!(GNOME_BANK_STAND.level, 1);
        assert_eq!(GNOME_BANK_STAIR_SOUTH.level, 0);

        let short = get("gnome_fletch_short").expect("gnome_fletch_short");
        assert_eq!(short.settings.start_script, Some("GnomeMagicChopper"));
        let inject = settings_inject_map(short.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(true)));
        let short_start = short
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let short_seed = short.steps[..short_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(short_seed.contains(&Proof::Stat {
            id: FLETCHING_STAT,
            min: 80,
        }));
        assert!(short_seed.contains(&Proof::StatAtMost {
            id: FLETCHING_STAT,
            max: 84,
        }));
        assert!(short_seed.contains(&Proof::ItemId {
            id: RUNE_AXE_ID,
            count: 1,
        }));
        assert!(short_seed.contains(&Proof::ItemIdAtMost {
            id: RUNE_AXE_ID,
            count: 1,
        }));
        assert!(short_seed.contains(&Proof::ItemId {
            id: KNIFE_ID,
            count: GNOME_BALLAST_KNIVES,
        }));
        assert!(short_seed.contains(&Proof::ItemIdAtMost {
            id: KNIFE_ID,
            count: GNOME_BALLAST_KNIVES,
        }));
        assert!(!short_seed.contains(&Proof::Stat {
            id: FLETCHING_STAT,
            min: 85,
        }));
        let short_watch = short.steps[short_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let unstrung_short = Proof::ItemId {
            id: UNSTRUNG_MAGIC_SHORTBOW_ID,
            count: 1,
        };
        assert_eq!(
            short_watch[0],
            Proof::StatXpGain {
                id: WOODCUTTING_STAT,
                min: 1
            }
        );
        assert_eq!(short_watch[1], logs);
        assert_eq!(
            short_watch[2],
            Proof::StatXpGain {
                id: FLETCHING_STAT,
                min: 1
            }
        );
        assert_eq!(short_watch[3], unstrung_short);
        assert!(short_watch.contains(&Proof::BankItemId {
            id: UNSTRUNG_MAGIC_SHORTBOW_ID,
            count: 1,
        }));
        assert!(!short_watch.contains(&Proof::BankItemId {
            id: UNSTRUNG_MAGIC_LONGBOW_ID,
            count: 1,
        }));
        assert!(!short_watch.contains(&Proof::ItemId {
            id: MAGIC_SHORTBOW_ID,
            count: 1,
        }));
        let first_product = short_watch
            .iter()
            .position(|arm| *arm == unstrung_short)
            .unwrap();
        let further_chop = short_watch.iter().rposition(|arm| *arm == logs).unwrap();
        assert!(first_product < further_chop);
        assert_eq!(short.proof, logs);

        let long = get("gnome_fletch_long").expect("gnome_fletch_long");
        let inject = settings_inject_map(long.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("fletchLogs"), Some(&Value::Bool(true)));
        let long_start = long
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let long_seed = long.steps[..long_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(long_seed.contains(&Proof::Stat {
            id: FLETCHING_STAT,
            min: 85,
        }));
        assert!(!long_seed.contains(&Proof::StatAtMost {
            id: FLETCHING_STAT,
            max: 84,
        }));
        let long_watch = long.steps[long_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(long_watch.contains(&Proof::ItemId {
            id: UNSTRUNG_MAGIC_LONGBOW_ID,
            count: 1,
        }));
        assert!(long_watch.contains(&Proof::BankItemId {
            id: UNSTRUNG_MAGIC_LONGBOW_ID,
            count: 1,
        }));
        assert!(!long_watch.contains(&unstrung_short));
        assert_eq!(long.proof, logs);

        let coal = get("coal_trucks").expect("coal_trucks");
        assert_eq!(coal.settings.start_script, Some("CoalTrucks"));
        assert_eq!(coal.settings.deadline, SCRIPT_GOLD_DEADLINE);
        assert!(coal.settings.script_settings_inject.is_none());
        let coal_start = coal
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let coal_seed = coal.steps[..coal_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(coal_seed.contains(&Proof::ArrivedNear {
            x: 2582,
            z: 3481,
            level: 0,
            radius: 8,
        }));
        assert!(coal_seed.contains(&Proof::Stat {
            id: MINING_STAT,
            min: 60,
        }));
        assert!(coal_seed.contains(&Proof::ItemId { id: 1275, count: 1 }));
        assert!(coal_seed.contains(&Proof::ItemId {
            id: KNIFE_ID,
            count: COAL_BALLAST_KNIVES,
        }));
        assert!(coal_seed.contains(&Proof::ItemIdAtMost {
            id: KNIFE_ID,
            count: COAL_BALLAST_KNIVES,
        }));
        assert!(coal_seed.contains(&Proof::ItemIdAtMost {
            id: COAL_ID,
            count: 0,
        }));
        let coal_watch = coal.steps[coal_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let coal_item = Proof::ItemId {
            id: COAL_ID,
            count: 1,
        };
        let truck = Proof::ArrivedNear {
            x: COAL_MINE_TRUCK_STAND.x,
            z: COAL_MINE_TRUCK_STAND.z,
            level: COAL_MINE_TRUCK_STAND.level,
            radius: 4,
        };
        assert_eq!(
            coal_watch,
            vec![
                Proof::StatXpGain {
                    id: MINING_STAT,
                    min: 1
                },
                coal_item,
                truck,
                Proof::ItemIdAtMost {
                    id: COAL_ID,
                    count: 0
                },
                coal_item,
            ]
        );
        assert!(!coal_watch.contains(&Proof::BankItemId {
            id: COAL_ID,
            count: 1,
        }));
        let first_coal = coal_watch.iter().position(|arm| *arm == coal_item).unwrap();
        let truck_i = coal_watch.iter().position(|arm| *arm == truck).unwrap();
        let further_coal = coal_watch
            .iter()
            .rposition(|arm| *arm == coal_item)
            .unwrap();
        assert!(first_coal < truck_i);
        assert!(truck_i < further_coal);
        assert_eq!(coal.proof, coal_item);

        for name in [
            "gnome_chop",
            "gnome_fletch_short",
            "gnome_fletch_long",
            "coal_trucks",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn station_production_cases_register_cook_smelt_and_spin_cycles() {
        let cook = get("cook_bot").expect("cook_bot");
        assert_eq!(cook.settings.start_script, Some("CookBot"));
        assert_eq!(cook.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(cook.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("fish"),
            Some(&Value::String("Raw salmon".into()))
        );
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Catherby".into()))
        );
        assert_eq!(inject.get("surface"), Some(&Value::String("Range".into())));
        let start = cook
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = cook.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2809,
            z: 3441,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: COOKING_STAT,
            min: COOKING_FIXTURE_LEVEL,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: RAW_SALMON_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: SALMON_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::BankItemId {
            id: RAW_SALMON_ID,
            count: COOK_RAW_SEED,
        }));
        let watch = cook.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let salmon = Proof::ItemId {
            id: SALMON_ID,
            count: 1,
        };
        let raw = Proof::ItemId {
            id: RAW_SALMON_ID,
            count: 1,
        };
        let range = Proof::ArrivedNear {
            x: CATHERBY_RANGE_STAND.x,
            z: CATHERBY_RANGE_STAND.z,
            level: CATHERBY_RANGE_STAND.level,
            radius: 8,
        };
        let cooking_xp = Proof::StatXpGain {
            id: COOKING_STAT,
            min: 1,
        };
        assert_eq!(watch[0], range);
        assert_eq!(watch[1], cooking_xp);
        assert_eq!(watch[2], salmon);
        let first_xp = watch.iter().position(|arm| *arm == cooking_xp).unwrap();
        let first_product = watch.iter().position(|arm| *arm == salmon).unwrap();
        let bank_product = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: SALMON_ID,
                    count: 1,
                }
            })
            .unwrap();
        let restock = watch.iter().position(|arm| *arm == raw).unwrap();
        let further = watch.iter().rposition(|arm| *arm == salmon).unwrap();
        assert!(first_xp < first_product);
        assert!(first_product < bank_product);
        assert!(bank_product < restock);
        assert!(restock < further);
        assert_ne!(first_product, further);
        assert_eq!(cook.proof, salmon);

        let lobster = get("cook_bot_lobster").expect("cook_bot_lobster");
        assert_eq!(lobster.settings.start_script, Some("CookBot"));
        let inject = settings_inject_map(lobster.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("fish"),
            Some(&Value::String("Raw lobster".into()))
        );
        assert_eq!(inject.get("surface"), Some(&Value::String("Range".into())));
        let lobster_start = lobster
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let lobster_watch = lobster.steps[lobster_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(lobster_watch.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: 1,
        }));
        assert!(lobster_watch.contains(&Proof::BankItemId {
            id: LOBSTER_ID,
            count: 1,
        }));
        assert!(!lobster_watch.contains(&salmon));
        assert_eq!(
            lobster.proof,
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 1,
            }
        );

        let bronze = get("smelter_bot").expect("smelter_bot");
        assert_eq!(bronze.settings.start_script, Some("SmelterBot"));
        let inject = settings_inject_map(bronze.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("bar"), Some(&Value::String("Bronze".into())));
        let bronze_start = bronze
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let bronze_seed = bronze.steps[..bronze_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(bronze_seed.contains(&Proof::Stat {
            id: SMITHING_STAT,
            min: 1,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: COPPER_ORE_ID,
            count: SMELT_ORE_SEED,
        }));
        assert!(bronze_seed.contains(&Proof::BankItemId {
            id: TIN_ORE_ID,
            count: SMELT_ORE_SEED,
        }));
        let bronze_watch = bronze.steps[bronze_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let bar = Proof::ItemId {
            id: BRONZE_BAR_ID,
            count: 1,
        };
        let smith_xp = Proof::StatXpGain {
            id: SMITHING_STAT,
            min: 1,
        };
        let first_xp = bronze_watch
            .iter()
            .position(|arm| *arm == smith_xp)
            .unwrap();
        let first_bar = bronze_watch.iter().position(|arm| *arm == bar).unwrap();
        assert!(first_xp < first_bar);
        assert!(bronze_watch.contains(&Proof::ArrivedNear {
            x: AL_KHARID_FURNACE.x,
            z: AL_KHARID_FURNACE.z,
            level: AL_KHARID_FURNACE.level,
            radius: 8,
        }));
        assert_eq!(bronze.proof, bar);

        let steel = get("smelter_bot_steel").expect("smelter_bot_steel");
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
            min: 30,
        }));
        assert!(steel_seed.contains(&Proof::BankItemId {
            id: IRON_ORE_ID,
            count: SMELT_ORE_SEED,
        }));
        assert!(steel_seed.contains(&Proof::BankItemId {
            id: COAL_ID,
            count: STEEL_COAL_SEED,
        }));
        let steel_watch = steel.steps[steel_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(steel_watch.contains(&Proof::ItemId {
            id: STEEL_BAR_ID,
            count: 1,
        }));
        assert!(!steel_watch.contains(&bar));
        assert_eq!(
            steel.proof,
            Proof::ItemId {
                id: STEEL_BAR_ID,
                count: 1,
            }
        );

        let spin = get("flax_spinner").expect("flax_spinner");
        assert_eq!(spin.settings.start_script, Some("FlaxSpinner"));
        assert!(spin.settings.script_settings_inject.is_none());
        let spin_start = spin
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let spin_seed = spin.steps[..spin_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(spin_seed.contains(&Proof::ArrivedNear {
            x: 2722,
            z: 3493,
            level: 0,
            radius: 8,
        }));
        assert!(spin_seed.contains(&Proof::Stat {
            id: CRAFTING_STAT,
            min: 1,
        }));
        assert!(spin_seed.contains(&Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        }));
        let spin_watch = spin.steps[spin_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let string = Proof::ItemId {
            id: BOW_STRING_ID,
            count: 1,
        };
        let craft_xp = Proof::StatXpGain {
            id: CRAFTING_STAT,
            min: 1,
        };
        let wheel = Proof::ArrivedNear {
            x: FLAX_SPINNER_WHEEL.x,
            z: FLAX_SPINNER_WHEEL.z,
            level: FLAX_SPINNER_WHEEL.level,
            radius: 8,
        };
        assert_eq!(spin_watch[0], wheel);
        assert_eq!(spin_watch[1], craft_xp);
        assert_eq!(spin_watch[2], string);
        assert_eq!(FLAX_SPINNER_WHEEL.level, 1);
        assert_eq!(spin.proof, string);

        for name in [
            "cook_bot",
            "cook_bot_lobster",
            "smelter_bot",
            "smelter_bot_steel",
            "flax_spinner",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn flax_aio_and_secondary_cases_register_collection_cycles() {
        let aio = get("flax_aio").expect("flax_aio");
        assert_eq!(aio.settings.start_script, Some("FlaxAIO"));
        assert_eq!(aio.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(aio.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("picking"), Some(&Value::Bool(true)));
        assert_eq!(inject.get("spinning"), Some(&Value::Bool(true)));
        let start = aio
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = aio.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2741,
            z: 3444,
            level: 0,
            radius: 6,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: CRAFTING_STAT,
            min: 1,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: FLAX_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        }));
        let watch = aio.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let flax = Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        };
        let string = Proof::ItemId {
            id: BOW_STRING_ID,
            count: 1,
        };
        let craft_xp = Proof::StatXpGain {
            id: CRAFTING_STAT,
            min: 1,
        };
        let wheel = Proof::ArrivedNear {
            x: FLAX_SPINNER_WHEEL.x,
            z: FLAX_SPINNER_WHEEL.z,
            level: FLAX_SPINNER_WHEEL.level,
            radius: 8,
        };
        let first_flax = watch.iter().position(|arm| *arm == flax).unwrap();
        let first_wheel = watch.iter().position(|arm| *arm == wheel).unwrap();
        let first_xp = watch.iter().position(|arm| *arm == craft_xp).unwrap();
        let first_string = watch.iter().position(|arm| *arm == string).unwrap();
        let bank_string = watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: BOW_STRING_ID,
                    count: 1,
                }
            })
            .unwrap();
        let further = watch.iter().rposition(|arm| *arm == flax).unwrap();
        assert!(first_flax < first_wheel);
        assert!(first_wheel < first_xp);
        assert!(first_xp < first_string);
        assert!(first_string < bank_string);
        assert!(bank_string < further);
        assert_ne!(first_flax, further);
        assert_eq!(aio.proof, flax);
        assert!(!watch.contains(&Proof::ItemId {
            id: BALL_OF_WOOL_ID,
            count: 1,
        }));

        let pick = get("flax_aio_pick").expect("flax_aio_pick");
        assert_eq!(pick.settings.start_script, Some("FlaxAIO"));
        let inject = settings_inject_map(pick.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("picking"), Some(&Value::Bool(true)));
        assert_eq!(inject.get("spinning"), Some(&Value::Bool(false)));
        let pick_start = pick
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let pick_watch = pick.steps[pick_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(pick_watch.contains(&Proof::ItemId {
            id: FLAX_ID,
            count: 28,
        }));
        assert!(pick_watch.contains(&Proof::BankItemId {
            id: FLAX_ID,
            count: 28,
        }));
        assert!(!pick_watch.contains(&string));
        assert!(!pick_watch.contains(&craft_xp));
        assert_eq!(
            pick.proof,
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            }
        );

        let spin = get("flax_aio_spin").expect("flax_aio_spin");
        assert_eq!(spin.settings.start_script, Some("FlaxAIO"));
        let inject = settings_inject_map(spin.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("picking"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("spinning"), Some(&Value::Bool(true)));
        let spin_start = spin
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let spin_seed = spin.steps[..spin_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(spin_seed.contains(&Proof::ArrivedNear {
            x: 2725,
            z: 3493,
            level: 0,
            radius: 8,
        }));
        assert!(spin_seed.contains(&Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        }));
        let spin_watch = spin.steps[spin_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert_eq!(spin_watch[0], wheel);
        assert_eq!(spin_watch[1], craft_xp);
        assert_eq!(spin_watch[2], string);
        assert_eq!(spin.proof, string);

        let eggs = get("herblore_secondaries").expect("herblore_secondaries");
        assert_eq!(eggs.settings.start_script, Some("HerbloreSecondaries"));
        let inject = settings_inject_map(eggs.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("secondary"),
            Some(&Value::String("Red spiders' eggs".into()))
        );
        let eggs_start = eggs
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let eggs_seed = eggs.steps[..eggs_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(eggs_seed.contains(&Proof::ArrivedNear {
            x: 3120,
            z: 9952,
            level: 0,
            radius: 8,
        }));
        assert!(eggs_seed.contains(&Proof::BankItemId {
            id: LOBSTER_ID,
            count: HERBLORE_EGG_FOOD_SEED,
        }));
        let seed_bank = eggs.steps[..eggs_start]
            .iter()
            .find(|step| {
                step.wait.arm
                    == Proof::BankItemId {
                        id: LOBSTER_ID,
                        count: HERBLORE_EGG_FOOD_SEED,
                    }
            })
            .expect("exact Edgeville seed-bank acknowledgement");
        assert!(matches!(seed_bank.kind, StepKind::Repeat { .. }));
        assert_eq!(
            EDGEVILLE_BANK_BOOTH,
            WorldTile {
                x: 3096,
                z: 3493,
                level: 0
            }
        );
        assert_eq!(EDGEVILLE_BANK_BOOTH_ID, 2213);
        assert_eq!(seed_bank.wait.budget_ticks, SCRIPT_GOLD_WATCH_TICKS);
        assert!(eggs_seed.contains(&Proof::LocActionNear {
            id: EDGEVILLE_BANK_BOOTH_ID,
            x: EDGEVILLE_BANK_BOOTH.x,
            z: EDGEVILLE_BANK_BOOTH.z,
            level: EDGEVILLE_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        }));
        assert!(eggs.steps[..eggs_start]
            .iter()
            .any(|step| step.name
                == "acknowledge exact Edgeville booth identity and Use-quickly action before bank send"));

        assert!(eggs_seed.contains(&Proof::ItemIdAtMost {
            id: RED_SPIDERS_EGGS_ID,
            count: 0,
        }));
        let eggs_watch = eggs.steps[eggs_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        let egg = Proof::ItemId {
            id: RED_SPIDERS_EGGS_ID,
            count: 1,
        };
        let newt = Proof::ItemId {
            id: EYE_OF_NEWT_ID,
            count: 1,
        };
        let first_egg = eggs_watch.iter().position(|arm| *arm == egg).unwrap();
        let bank_egg = eggs_watch
            .iter()
            .position(|arm| {
                *arm == Proof::BankItemId {
                    id: RED_SPIDERS_EGGS_ID,
                    count: 1,
                }
            })
            .unwrap();
        let further_egg = eggs_watch.iter().rposition(|arm| *arm == egg).unwrap();
        assert!(first_egg < bank_egg);
        assert!(bank_egg < further_egg);
        assert!(!eggs_watch.contains(&newt));
        assert_eq!(eggs.proof, egg);

        let buy = get("herblore_secondaries_newt").expect("herblore_secondaries_newt");
        assert_eq!(buy.settings.start_script, Some("HerbloreSecondaries"));
        let inject = settings_inject_map(buy.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("secondary"),
            Some(&Value::String("Eye of newt".into()))
        );
        let buy_start = buy
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let buy_seed = buy.steps[..buy_start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(buy_seed.contains(&Proof::ArrivedNear {
            x: 3012,
            z: 3259,
            level: 0,
            radius: 6,
        }));
        assert!(buy_seed.contains(&Proof::BankItemId {
            id: COINS_ID,
            count: HERBLORE_NEWT_COIN_SEED,
        }));
        let buy_watch = buy.steps[buy_start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(buy_watch.contains(&newt));
        assert!(buy_watch.contains(&Proof::BankItemId {
            id: EYE_OF_NEWT_ID,
            count: 1,
        }));
        assert!(!buy_watch.contains(&egg));
        assert_eq!(buy.proof, newt);

        for name in [
            "flax_aio",
            "flax_aio_pick",
            "flax_aio_spin",
            "herblore_secondaries",
            "herblore_secondaries_newt",
        ] {
            assert!(names().contains(&name));
        }
    }

    #[test]
    fn combat_core_cases_register_melee_cycles() {
        let strength = Proof::StatXpGain {
            id: STRENGTH_STAT,
            min: 1,
        };
        let attack = Proof::Stat {
            id: 0,
            min: COMBAT_ATTACK_LEVEL,
        };
        let chaos = get("chaos_druid").expect("chaos_druid");
        assert_eq!(chaos.settings.start_script, Some("ChaosDruidKiller"));
        assert_eq!(chaos.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(chaos.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Edgeville Dungeon".into()))
        );
        assert_eq!(
            inject.get("combatStyleIndex"),
            Some(&Value::String("1".into()))
        );
        let start = chaos
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = chaos.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3110,
            z: 9936,
            level: 0,
            radius: 14,
        }));
        assert!(seed.contains(&attack));
        assert!(seed.contains(&Proof::Stat {
            id: STRENGTH_STAT,
            min: COMBAT_ATTACK_LEVEL,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: CHAOS_DRUID_FOOD,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: COMBAT_SCIMITAR_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: UNIDENTIFIED_GUAM_ID,
            count: 0,
        }));
        assert_eq!(chaos.steps[start + 1].wait.arm, strength);
        assert_eq!(chaos.proof, strength);

        let tower = get("chaos_druid_tower").expect("chaos_druid_tower");
        assert_eq!(tower.settings.start_script, Some("ChaosDruidKiller"));
        let inject = settings_inject_map(tower.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Chaos Druid Tower".into()))
        );
        assert_eq!(
            inject.get("combatStyleIndex"),
            Some(&Value::String("1".into()))
        );
        let start = tower
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = tower.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2562,
            z: 3356,
            level: 0,
            radius: 4,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 46,
        }));
        assert_eq!(tower.proof, strength);

        let yanille = get("chaos_druid_yanille").expect("chaos_druid_yanille");
        assert_eq!(yanille.settings.start_script, Some("ChaosDruidKiller"));
        let inject = settings_inject_map(yanille.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("location"),
            Some(&Value::String("Yanille Dungeon".into()))
        );
        let start = yanille
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = yanille.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2580,
            z: 9501,
            level: 0,
            radius: 8,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: AGILITY_STAT,
            min: 40,
        }));
        assert_eq!(yanille.proof, strength);

        let moss = get("moss_giant").expect("moss_giant");
        assert_eq!(moss.settings.start_script, Some("MossGiant"));
        let inject = settings_inject_map(moss.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("melee".into()))
        );
        assert_eq!(
            inject.get("meleeStyle"),
            Some(&Value::String("strength".into()))
        );
        assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
        let start = moss
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = moss.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2553,
            z: 3406,
            level: 0,
            radius: 10,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: MOSS_GIANT_FOOD,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BIG_BONES_ID,
            count: 0,
        }));
        assert_eq!(moss.proof, strength);

        let hill = get("hill_giant").expect("hill_giant");
        assert_eq!(hill.settings.start_script, Some("HillGiant"));
        let inject = settings_inject_map(hill.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("meleeStyle"),
            Some(&Value::String("strength".into()))
        );
        assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
        assert!(!inject.contains_key("weapon"));
        let start = hill
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = hill.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3110,
            z: 9832,
            level: 0,
            radius: 16,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: TROUT_ID,
            count: HILL_GIANT_FOOD,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BIG_BONES_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: LIMPWURT_ROOT_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: BRASS_KEY_ID,
            count: 1,
        }));
        assert_eq!(hill.proof, strength);

        let auto = get("auto_fighter").expect("auto_fighter");
        assert_eq!(auto.settings.start_script, Some("AutoFighter"));
        let inject = settings_inject_map(auto.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
        assert_eq!(
            inject.get("spot"),
            Some(&Value::String("Start position".into()))
        );
        assert_eq!(inject.get("banking"), Some(&Value::String("None".into())));
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
        let start = auto
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = auto.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2661,
            z: 3306,
            level: 0,
            radius: 8,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: TROUT_ID,
            count: AUTO_FIGHTER_FOOD,
        }));
        assert_eq!(auto.proof, strength);

        for name in [
            "chaos_druid",
            "chaos_druid_tower",
            "chaos_druid_yanille",
            "moss_giant",
            "hill_giant",
            "auto_fighter",
            "rock_crab",
            "green_dragon",
            "fire_giant",
            "ardy_fighter",
        ] {
            assert!(names().contains(&name));
            let scenario = get(name).unwrap();
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
            assert!(!scenario
                .steps
                .iter()
                .any(|step| matches!(step.wait.arm, Proof::BankItemId { .. })));
        }
    }

    #[test]
    fn remaining_fighter_core_cases_register_melee_cycles() {
        let strength = Proof::StatXpGain {
            id: STRENGTH_STAT,
            min: 1,
        };
        let rock = get("rock_crab").expect("rock_crab");
        assert_eq!(rock.settings.start_script, Some("RockCrab"));
        assert_eq!(rock.settings.deadline, SCRIPT_GOLD_DEADLINE);
        let inject = settings_inject_map(rock.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("melee".into()))
        );
        assert_eq!(
            inject.get("meleeStyle"),
            Some(&Value::String("strength".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("bankStrategy"),
            Some(&Value::String("Off".into()))
        );
        let start = rock
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = rock.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2712,
            z: 3688,
            level: 0,
            radius: 2,
        }));
        assert!(rock.steps[..start].iter().any(
            |step| step.name == "acknowledge dormant Rocks in the supported field before Start"
        ));
        assert!(seed.contains(&Proof::ItemId {
            id: LOBSTER_ID,
            count: ROCK_CRAB_FOOD,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: COMBAT_SCIMITAR_ID,
            count: 1,
        }));
        assert_eq!(rock.steps[start + 1].wait.arm, strength);
        assert_eq!(rock.proof, strength);

        let dragon = get("green_dragon").expect("green_dragon");
        assert_eq!(dragon.settings.start_script, Some("GreenDragon"));
        let inject = settings_inject_map(dragon.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("usePotions"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("escape"),
            Some(&Value::String("Flee to bank".into()))
        );
        assert_eq!(
            inject.get("weapon"),
            Some(&Value::String("Rune scimitar".into()))
        );
        assert_eq!(
            inject.get("shield"),
            Some(&Value::String("Dragonfire shield".into()))
        );
        let start = dragon
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = dragon.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 3096,
            z: 3814,
            level: 0,
            radius: 22,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: RUNE_SCIMITAR_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::EquipmentId {
            id: DRAGONFIRE_SHIELD_ID,
        }));
        assert!(dragon.steps[..start].iter().any(|step| step.name
            == "wear and acknowledge Dragonfire shield before hostile-field teleport"));

        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: DRAGON_BONES_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: GREEN_DRAGONHIDE_ID,
            count: 0,
        }));
        assert_eq!(dragon.proof, strength);

        let giant = get("fire_giant").expect("fire_giant");
        assert_eq!(giant.settings.start_script, Some("FireGiant"));
        let inject = settings_inject_map(giant.settings.script_settings_inject).unwrap();
        assert_eq!(
            inject.get("escapeTele"),
            Some(&Value::String("Barrel (free)".into()))
        );
        assert_eq!(inject.get("buryBones"), Some(&Value::Bool(false)));
        assert!(giant.steps.iter().any(|step| matches!(
            step.wait.arm,
            Proof::QuestDone {
                name: "Waterfall Quest"
            }
        )));
        let start = giant
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = giant.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2575,
            z: 9893,
            level: 0,
            radius: 10,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: GLARIALS_AMULET_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemId {
            id: ROPE_ID,
            count: 1,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BIG_BONES_ID,
            count: 0,
        }));
        assert_eq!(giant.proof, strength);

        let ardy = get("ardy_fighter").expect("ardy_fighter");
        assert_eq!(ardy.settings.start_script, Some("ArdyFighter"));
        let inject = settings_inject_map(ardy.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("strength".into()))
        );
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("bankStrategy"),
            Some(&Value::String("Off".into()))
        );
        assert_eq!(inject.get("foodTarget"), Some(&Value::from(1.0)));
        let start = ardy
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let seed = ardy.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(seed.contains(&Proof::ArrivedNear {
            x: 2661,
            z: 3306,
            level: 0,
            radius: 12,
        }));
        assert!(seed.contains(&Proof::Stat {
            id: THIEVING_STAT,
            min: 5,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CAKE_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: BREAD_ID,
            count: 0,
        }));
        assert!(seed.contains(&Proof::ItemIdAtMost {
            id: CHOCOLATE_SLICE_ID,
            count: 0,
        }));
        assert!(!seed.contains(&Proof::ItemId {
            id: CAKE_ID,
            count: 1,
        }));
        assert_eq!(ardy.proof, strength);
    }

    #[test]
    fn auto_fighter_mage_prepares_and_observes_real_autocast_combat() {
        let mage = get("auto_fighter_mage").expect("auto_fighter_mage");
        assert_eq!(mage.settings.start_script, Some("AutoFighter"));
        assert_eq!(mage.settings.deadline, SCRIPT_GOLD_DEADLINE);

        let inject = settings_inject_map(mage.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("target"), Some(&Value::String("Guard".into())));
        assert_eq!(
            inject.get("spot"),
            Some(&Value::String("Start position".into()))
        );
        assert_eq!(
            inject.get("combatStyle"),
            Some(&Value::String("mage".into()))
        );
        assert_eq!(
            inject.get("spell"),
            Some(&Value::String("Fire Strike".into()))
        );
        assert_eq!(inject.get("runesWithdraw"), Some(&Value::from(150.0)));
        assert_eq!(inject.get("banking"), Some(&Value::String("None".into())));
        assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));

        let start = mage
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let before = mage.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(before.contains(&Proof::Stat { id: 6, min: 13 }));
        assert!(before.contains(&Proof::Stat { id: 3, min: 40 }));
        assert!(before.contains(&Proof::ItemId { id: 333, count: 8 }));
        assert!(before.contains(&Proof::ItemId {
            id: 558,
            count: 150,
        }));
        assert!(before.contains(&Proof::ItemId {
            id: 556,
            count: 300,
        }));
        assert!(before.contains(&Proof::EquipmentId { id: 1387 }));
        assert!(before.contains(&Proof::ArrivedNear {
            x: 2661,
            z: 3306,
            level: 0,
            radius: 8,
        }));

        let after = mage.steps[start + 1..]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(after.contains(&Proof::Varp { id: 108, min: 3 }));
        assert!(after.contains(&Proof::StatXpGain { id: 6, min: 1 }));
        assert!(after.contains(&Proof::ItemIdAtMost {
            id: 558,
            count: 149,
        }));
        assert!(after.contains(&Proof::ItemIdAtMost {
            id: 556,
            count: 298,
        }));
        assert_eq!(mage.proof, Proof::StatXpGain { id: 6, min: 1 });
    }

    #[test]
    fn ranged_and_consumable_options_prepare_the_exact_frozen_script_branches() {
        let ranged_xp = Proof::StatXpGain { id: 4, min: 1 };
        for (name, card, food_id, food_count) in [
            ("auto_fighter_range", "AutoFighter", 333, 8),
            ("rock_crab_range", "RockCrab", 379, 8),
        ] {
            let scenario = get(name).unwrap_or_else(|| panic!("{name} registered"));
            assert_eq!(scenario.settings.start_script, Some(card));
            assert_eq!(scenario.settings.deadline, SCRIPT_GOLD_DEADLINE);
            let inject = settings_inject_map(scenario.settings.script_settings_inject).unwrap();
            assert_eq!(
                inject.get("combatStyle"),
                Some(&Value::String("range".into()))
            );
            assert_eq!(
                inject.get("rangeStyle"),
                Some(&Value::String("rapid".into()))
            );
            assert_eq!(
                inject.get("ammo"),
                Some(&Value::String("Bronze arrow".into()))
            );
            assert_eq!(inject.get("solveClues"), Some(&Value::Bool(false)));
            assert_eq!(
                inject.get("useSpecial"),
                (name == "auto_fighter_range").then_some(&Value::Bool(false))
            );
            assert!(!inject.contains_key("weapon"));

            let start = scenario
                .steps
                .iter()
                .position(|step| matches!(step.kind, StepKind::StartScript))
                .unwrap();
            let before = scenario.steps[..start]
                .iter()
                .map(|step| step.wait.arm)
                .collect::<Vec<_>>();
            assert!(before.contains(&Proof::Stat { id: 4, min: 40 }));
            assert!(before.contains(&Proof::ItemId {
                id: food_id,
                count: food_count
            }));
            assert!(before.contains(&Proof::EquipmentId { id: 853 }));
            assert!(before.contains(&Proof::EquipmentId { id: 882 }));
            assert!(scenario.steps[start + 1..]
                .iter()
                .any(|step| step.wait.arm == ranged_xp));
            assert_eq!(scenario.proof, ranged_xp);
        }

        let rock = get("rock_crab_range").expect("rock crab range");
        let rock_inject = settings_inject_map(rock.settings.script_settings_inject).unwrap();
        assert_eq!(
            rock_inject.get("bow"),
            Some(&Value::String("Maple shortbow".into()))
        );
        let rock_start = rock
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(rock.steps[..rock_start].iter().any(
            |step| step.name == "acknowledge dormant Rocks in the supported field before Start"
        ));

        let special = get("green_dragon_special").expect("green dragon special");
        let inject = settings_inject_map(special.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(true)));
        assert_eq!(inject.get("usePotions"), Some(&Value::Bool(false)));
        assert_eq!(
            inject.get("weapon"),
            Some(&Value::String("Dragon dagger".into()))
        );
        let start = special
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let before = special.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(before.contains(&Proof::Stat { id: 0, min: 60 }));
        assert!(before.contains(&Proof::EquipmentId { id: 1215 }));
        assert!(before.contains(&Proof::EquipmentId { id: 1540 }));
        // The card refuses to arm below the wielded weapon's cost, so the
        // prepared pool is an acknowledged prerequisite, not a post-Start fix.
        assert!(before.contains(&Proof::Varp { id: 300, min: 250 }));

        let potions = get("green_dragon_potions").expect("green dragon potions");
        let inject = settings_inject_map(potions.settings.script_settings_inject).unwrap();
        assert_eq!(inject.get("useSpecial"), Some(&Value::Bool(false)));
        assert_eq!(inject.get("usePotions"), Some(&Value::Bool(true)));
        let start = potions
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        let before = potions.steps[..start]
            .iter()
            .map(|step| step.wait.arm)
            .collect::<Vec<_>>();
        assert!(before.contains(&Proof::ItemId { id: 145, count: 1 }));
        assert!(before.contains(&Proof::ItemId { id: 157, count: 1 }));
        assert!(before.contains(&Proof::EquipmentId { id: 1540 }));
        assert!(before.contains(&Proof::ItemIdAtMost { id: 147, count: 0 }));
        assert!(before.contains(&Proof::ItemIdAtMost { id: 159, count: 0 }));

        for name in [
            "auto_fighter_range",
            "rock_crab_range",
            "green_dragon_special",
            "green_dragon_potions",
        ] {
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
            "rune_crafter",
            "rune_crafter_earth",
            "mule_crafter",
            "ardy_cakes",
            "ardy_thiever",
            "ardy_thiever_knight",
            "gnome_chop",
            "gnome_fletch_short",
            "gnome_fletch_long",
            "coal_trucks",
            "cook_bot",
            "cook_bot_lobster",
            "smelter_bot",
            "smelter_bot_steel",
            "flax_spinner",
            "flax_aio",
            "flax_aio_pick",
            "flax_aio_spin",
            "herblore_secondaries",
            "herblore_secondaries_newt",
            "chaos_druid",
            "moss_giant",
            "hill_giant",
            "auto_fighter",
            "auto_fighter_mage",
            "rock_crab",
            "green_dragon",
            "fire_giant",
            "ardy_fighter",
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
