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
pub mod fixture;
pub mod proof;
mod render_betty_views;
mod runner;
pub mod shot;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

use api::interact::{
    cheat, logout, op_loc, tele_args, ActionSpec, Driver, Interactions, OpTarget, SendReason,
    SendResult, MAXME_SETSTATS,
};
use api::snapshot::{GameSnapshot, ReadContext, WorldTile};
use client::client::Client;
use serde_json::{Map, Value};

pub use evidence::{Evidence, InvRow, StatRow};
pub use fixture::{
    apply_fixture_mode, as_run_prepared, default_fixture_path, default_fixture_sav_dir,
    fixture_prereqs_of, fixture_preset_for, harness_writer_script, prepare_offline_fixture,
    run_prepared_has_setup_cheats, FixtureAccount, FixtureIdentity, FixtureMode,
    OfflinePrepareOpts,
};
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
    /// Exact in-tree example file name (`bone_burier_v2.ts` / `.js`). When
    /// set, live prepare Loads that path as a File card and selects it by
    /// canonical-path `identity_id()` — never a shared stem. Distinct from
    /// [`Self::start_script`] catalog / TradeBot fixture names.
    pub start_file: Option<&'static str>,
    /// After game-state proofs pass, headed/TUI live waits for this
    /// existing isolate self-stop receipt reason plus Idle before treating
    /// the run as complete. Not a game-chat predicate.
    pub wait_script_stop: Option<&'static str>,
    /// Scenario-only parameter overrides merged last at script Start (never
    /// written to operator `script-settings.json`).
    pub script_settings_inject: Option<&'static [ScriptSettingInject]>,
    /// When set (e.g. `"partner"`), live_prepare inserts the minted
    /// companion username (`names[1]`) into the script settings bag under
    /// this key after [`script_settings_inject`].
    pub inject_companion_as: Option<&'static str>,
    /// Explicit world proofs prepare must acknowledge and run-prepared must
    /// observe before Start. When `None`, [`fixture_prereqs_of`] derives them
    /// from pre-StartScript wait arms (position/stat/item-like only).
    pub fixture_prereqs: Option<&'static [Proof]>,
    /// Harness-owned loadouts posted at catalog Start instead of reading
    /// operator `loadouts.json`. Scenario-scoped; not a global name reservation
    /// on profile Start/reload.
    pub fixture_loadouts: Option<&'static [FixtureLoadout]>,
}

/// One explicit loadout a live harness posts at catalog Start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureLoadout {
    pub name: &'static str,
    pub carry: &'static [(&'static str, u32)],
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
            start_file: None,
            wait_script_stop: None,
            script_settings_inject: None,
            inject_companion_as: None,
            fixture_prereqs: None,
            fixture_loadouts: None,
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
        name if render_betty_views::NAMES.contains(&name) => render_betty_views::get(name),
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
        "bone_burier_v2_ts" => Some(bone_burier_v2_scenario(
            "bone_burier_v2_ts",
            "bone_burier_v2.ts",
        )),
        "bone_burier_v2_js" => Some(bone_burier_v2_scenario(
            "bone_burier_v2_js",
            "bone_burier_v2.js",
        )),
        "strange_plant_owned" => Some(strange_plant_owned_scenario()),
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
        "alcher_low" => Some(alcher_low_scenario()),
        "alcher_fire_battlestaff" => Some(alcher_fire_battlestaff_scenario()),
        "alcher_swarm_drain" => Some(alcher_swarm_drain_scenario()),
        "bank_fletcher" => Some(bank_fletcher_scenario()),
        "bank_fletcher_shafts" => Some(bank_fletcher_shafts_scenario()),
        "bank_fletcher_headless" => Some(bank_fletcher_headless_scenario()),
        "bank_fletcher_string" => Some(bank_fletcher_string_scenario()),
        "bank_fletcher_cut_string" => Some(bank_fletcher_cut_string_scenario()),
        "dart_fletcher" => Some(dart_fletcher_scenario()),
        "dart_fletcher_iron" => Some(dart_fletcher_iron_scenario()),
        "herb_cleaner" => Some(herb_cleaner_scenario()),
        "herb_cleaner_named" => Some(herb_cleaner_named_scenario()),
        "herb_cleaner_empty_bank" => Some(herb_cleaner_empty_bank_scenario()),
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
        "superheater_silver_low_natures" => Some(superheater_silver_low_natures_scenario()),
        "superheater_mithril" => Some(superheater_mithril_scenario()),
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
        "rock_crab_bank" => Some(rock_crab_bank_scenario()),
        "green_dragon_bank" => Some(green_dragon_bank_scenario()),
        "green_dragon_tele" => Some(green_dragon_tele_scenario()),
        "fire_giant_approach" => Some(fire_giant_approach_scenario()),
        "fire_giant_bank" => Some(fire_giant_bank_scenario()),
        "aio_teleport" => Some(aio_teleport_scenario()),
        "aio_teleport_falador" => Some(aio_teleport_falador_scenario()),
        "aio_teleport_no_staff" => Some(aio_teleport_no_staff_scenario()),
        "shop_buyout" => Some(shop_buyout_scenario()),
        "shop_buyout_aubury" => Some(shop_buyout_aubury_scenario()),
        "shop_buyout_lowe" => Some(shop_buyout_lowe_scenario()),
        "shop_buyout_hickton" => Some(shop_buyout_hickton_scenario()),
        "shop_buyout_harry" => Some(shop_buyout_harry_scenario()),
        "shop_buyout_betty" => Some(shop_buyout_betty_scenario()),
        "shop_buyout_gerrant" => Some(shop_buyout_gerrant_scenario()),
        "shop_buyout_bob" => Some(shop_buyout_bob_scenario()),
        "shop_buyout_nurmof" => Some(shop_buyout_nurmof_scenario()),
        "shop_buyout_magic" => Some(shop_buyout_magic_scenario()),
        "shop_buyout_lundail" => Some(shop_buyout_lundail_scenario()),
        "shop_buyout_fernahei" => Some(shop_buyout_fernahei_scenario()),
        "smithing_bot" => Some(smithing_bot_scenario()),
        "smithing_bot_platebody" => Some(smithing_bot_platebody_scenario()),
        "smithing_bot_nails" => Some(smithing_bot_nails_scenario()),
        "smithing_bot_mithril" => Some(smithing_bot_mithril_scenario()),
        "leather_crafter" => Some(leather_crafter_scenario()),
        "leather_crafter_hard_body" => Some(leather_crafter_hard_body_scenario()),
        "leather_crafter_green_body" => Some(leather_crafter_green_body_scenario()),
        "leather_crafter_chaps" => Some(leather_crafter_chaps_scenario()),
        "leather_crafter_thread_shop" => Some(leather_crafter_thread_shop_scenario()),
        "firemaker" => Some(firemaker_scenario()),
        "firemaker_oak" => Some(firemaker_oak_scenario()),
        "climbing_boots" => Some(climbing_boots_scenario()),
        "climbing_boots_teleport" => Some(climbing_boots_teleport_scenario()),
        "script_trade" => Some(script_trade_scenario()),
        "nature_crafter_air" => Some(nature_crafter_air_scenario()),
        "mule_crafter_air" => Some(mule_crafter_air_scenario()),
        "flax_runner" => Some(flax_runner_scenario()),
        "duel_arena" => Some(duel_arena_scenario()),
        _ => None,
    }
}

/// Every registered scenario name (for the `--live script_<name>` usage).
pub fn names() -> Vec<&'static str> {
    vec![
        "walk",
        "render_smoke",
        "render_betty_views_betty_yaw0",
        "render_betty_views_betty_yaw512",
        "render_betty_views_falador_street_yaw0",
        "render_betty_views_falador_street_yaw512",
        "render_betty_views_west_bank_yaw0",
        "render_betty_views_west_bank_yaw512",
        "render_betty_views_dwarven_wall_yaw0",
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
        "bone_burier_v2_ts",
        "bone_burier_v2_js",
        "strange_plant_owned",
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
        "alcher_low",
        "alcher_fire_battlestaff",
        "alcher_swarm_drain",
        "bank_fletcher",
        "bank_fletcher_shafts",
        "bank_fletcher_headless",
        "bank_fletcher_string",
        "bank_fletcher_cut_string",
        "dart_fletcher",
        "dart_fletcher_iron",
        "herb_cleaner",
        "herb_cleaner_named",
        "herb_cleaner_empty_bank",
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
        "superheater_silver_low_natures",
        "superheater_mithril",
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
        "rock_crab_bank",
        "green_dragon_bank",
        "green_dragon_tele",
        "fire_giant_approach",
        "fire_giant_bank",
        "aio_teleport",
        "aio_teleport_falador",
        "aio_teleport_no_staff",
        "shop_buyout",
        "shop_buyout_aubury",
        "shop_buyout_lowe",
        "shop_buyout_hickton",
        "shop_buyout_harry",
        "shop_buyout_betty",
        "shop_buyout_gerrant",
        "shop_buyout_bob",
        "shop_buyout_nurmof",
        "shop_buyout_magic",
        "shop_buyout_lundail",
        "shop_buyout_fernahei",
        "smithing_bot",
        "smithing_bot_platebody",
        "smithing_bot_nails",
        "smithing_bot_mithril",
        "leather_crafter",
        "leather_crafter_hard_body",
        "leather_crafter_green_body",
        "leather_crafter_chaps",
        "leather_crafter_thread_shop",
        "firemaker",
        "firemaker_oak",
        "climbing_boots",
        "climbing_boots_teleport",
        "script_trade",
        "nature_crafter_air",
        "mule_crafter_air",
        "flax_runner",
        "duel_arena",
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
            terminal_shot: Some("nav_paint_path terminal"),
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

/// Verified packed Varrock East bank tile used by the v2 writer preset and login gate.
const BONE_BURIER_V2_BANK: WorldTile = WorldTile {
    x: 3253,
    z: 3421,
    level: 0,
};

/// Isolate self-stop reason from `crates/script/examples/bone_burier_v2.ts`.
const BONE_BURIER_V2_STOP_REASON: &str = "confirmed loaded current-generation bank exhaustion";

const BONE_BURIER_V2_PREREQS: &[Proof] = &[
    Proof::Item {
        name: "Bones",
        count: 5,
    },
    Proof::SideTabAvailable { index: 3 },
    Proof::ArrivedNear {
        x: BONE_BURIER_V2_BANK.x,
        z: BONE_BURIER_V2_BANK.z,
        level: BONE_BURIER_V2_BANK.level,
        radius: 8,
    },
];

/// Bury 5 + restock 28 + bury 28 + reopen bank + clean self-stop.
const BONE_BURIER_V2_DEADLINE: Duration = Duration::from_secs(360);
const BONE_BURIER_V2_WATCH_TICKS: u32 = 240;

/// Headed File-card BoneBurier v2: one exact in-tree example per scenario id.
/// v1 [`bone_burier_scenario`] stays the catalog compatibility witness.
fn bone_burier_v2_scenario(name: &'static str, file_name: &'static str) -> Scenario {
    let watch = |step_name, arm| Step {
        name: step_name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: BONE_BURIER_V2_WATCH_TICKS,
            arm,
        },
    };
    let later_bank = Proof::BankItemAtMost {
        name: "Bones",
        count: 0,
    };
    Scenario {
        name,
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
                name: "tele to the verified Varrock East bank tile",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(
                            c,
                            &tele_args(
                                BONE_BURIER_V2_BANK.level,
                                BONE_BURIER_V2_BANK.x,
                                BONE_BURIER_V2_BANK.z,
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: BONE_BURIER_V2_BANK.x,
                        z: BONE_BURIER_V2_BANK.z,
                        level: BONE_BURIER_V2_BANK.level,
                        radius: 8,
                    },
                    budget_ticks: 120,
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
            watch(
                "watch the replenished pack yield further Prayer XP",
                Proof::FreshStatXpGain { id: 5, min: 1 },
            ),
            watch(
                "watch the replenished pack run out",
                Proof::ItemAtMost {
                    name: "Bones",
                    count: 0,
                },
            ),
            watch(
                "watch a later loaded current bank still at zero",
                later_bank,
            ),
        ],
        proof: later_bank,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: BONE_BURIER_V2_DEADLINE,
            start_script: None,
            start_file: Some(file_name),
            wait_script_stop: Some(BONE_BURIER_V2_STOP_REASON),
            terminal_shot: Some(name),
            fixture_prereqs: Some(BONE_BURIER_V2_PREREQS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Shared live-script seed: stick `tutorial=1000`, relog so side tab 3 binds.
pub(crate) fn script_live_seed_steps() -> Vec<Step> {
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

/// One real server-owned Strange Plant lifecycle. The upstream debugproc
/// spawns `macro_triffidseed` for the logged-in player; the shared Rust
/// guardian must probe the otherwise featureless adjacent seed, authenticate
/// the canonical growing response, retry without click spam, then pick the
/// ripe fruit. A send or growing message alone is not terminal evidence.
fn strange_plant_owned_scenario() -> Scenario {
    let fruit = Proof::Item {
        name: "Strange fruit",
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    // Use the existing inert TradeBot file fixture so run-prepared can retain
    // the real post-Start macro injection while stripping every setup cheat.
    // An empty partner makes the fixture passive; the Rust guardian owns the
    // random lifecycle under test.
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "spawn the upstream owned Strange Plant",
        kind: StepKind::Perform {
            send: Box::new(|c, _| cheat(c, "~macro_event 5")),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "The fruit isn't ready to be picked yet",
            },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "watch the authenticated plant ripen and yield its fruit",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: fruit,
            budget_ticks: 180,
        },
    });
    Scenario {
        name: "strange_plant_owned",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: fruit,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(150),
            start_script: Some("TradeBot"),
            terminal_shot: Some("strange_plant_owned"),
            fixture_prereqs: Some(THIEVER_FIXTURE_PREREQS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Explicit `speed 600` so a leftover nav `speed 300` is not inherited.
fn gold_script_nav() -> ScenarioNav {
    ScenarioNav::default().with_tick_ms(600)
}

/// Ordinary post-Start watch budget in **runner dirty-snapshot increments**
/// (`runner`: `ticks_waited += 1` when `snapshot.rebuild` reports any family
/// gen moved). Not engine `World.TICKRATE` (600ms) game ticks and not wall
/// seconds — do not convert as 150×600ms.
const SCRIPT_GOLD_WATCH_TICKS: u32 = 150;
/// Seed + tele + drain + the short watch. Not a 6-minute soak. Wall-clock
/// whole-scenario cap (independent of dirty-tick budgets).
const SCRIPT_GOLD_DEADLINE: Duration = Duration::from_secs(180);
/// SmithingBot product→fresh-bank watch after the first pack item appears.
///
/// **Units:** this constant is a `budget_ticks` / runner dirty-snapshot
/// count (`runner.rs`: increment only when `snapshot.rebuild` is dirty —
/// any family gen moved). It is **not** an engine `p_delay` game-tick
/// count and not wall time. Capture `tick` advances only on PLAYER_INFO;
/// live fail at cf41 had outcome total_ticks 204 / 102.075s wall with
/// snapshot `tick` 173 — those figures are not interchangeable proofs.
///
/// **Engine lower bound (separate unit — selected289 `smithing_anvil`):**
/// `p_delay(1)` + work + `p_delay(2)`; `P_DELAY` → `delayedUntil =
/// currentTick + 1 + n` → 2+3 = **5 engine ticks/bar**. `makeFromPanelMax`
/// → Make-10 (op 3). Hammer kept → ≤27 bars/trip; deposit arms on first
/// product → ≤26 remain → 26×5 = **130 engine ticks** pure forge.
/// Bronze dagger 12.5 xp; RS thresholds 83 / 174 / 276 → level 1→2, 2→3,
/// and **3→4** all land inside a 27-bar first trip (3→4 still in the
/// remaining ≤7 bars after measured 20 daggers / 250 xp / level 3). Each
/// level-up plus a Make-10 residual restart needs ContinueDialog + re-Use
/// anvil + panel (~15 engine ticks × 4 interrupts ≈ 60). Anvil
/// (3188,3425) → VW bank Chebyshev 15 + open/deposit ≈ 30 engine ticks.
/// Engine LB ≈ 130+60+30 = **220 engine ticks** (not assigned raw to this
/// constant).
///
/// **Dirty budget:** measured deposit step exhausted 150 dirties still
/// mid-craft (20×1205 + 7 bars, anim 898, three MakePanel op3 +
/// ContinueDialog pairs, no deposit) — progressive craft, not a freeze.
/// Residual at ~8 dirties/item (interrupted rate) × 7 bars + L3→4 + bank
/// walk pushes arm-to-deposit past 220 dirties; pad explicitly for
/// dirty≠engine and match the existing bone_burier bank-item pattern →
/// **240**. Diagnosis remains **provisional** until root LIVE confirms
/// deposit/restock/return/fresh XP inside unchanged 180s deadline.
/// Global [`SCRIPT_GOLD_DEADLINE`] (180s) and other gold watches stay 150.
const SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS: u32 = 240;
/// HerbloreSecondaries eggs: first-Take → fresh bank 223, then dungeon return.
///
/// **Units:** `budget_ticks` are runner dirty-snapshot increments
/// (`runner.rs`: `ticks_waited += 1` only when `snapshot.rebuild` is dirty).
/// Not engine `World.TICKRATE` (600ms) ticks and not wall seconds. Capture
/// `tick` advances on PLAYER_INFO. live7wnm9z_0: outcome 170 dirties /
/// 82.065s wall / snapshot `tick` 141 — those figures are not interchangeable.
///
/// **Observed exhaustion of the default 150-dirty watch:** step 29
/// `fresh_bank_item_id(223)>=1` exhausted 150 dirties still in the field.
/// Terminal 3117,9951, inv 6×223 + 6×379, bank closed gen 6, paint
/// Got 6 / Trips 0 / Food 6, status taking eggs, in combat HP 6/10.
/// Canonical `needsRestock` is false until packFull or
/// (`takeFood && foodWant>0 && foodCount<1`). foodWithdraw default 10 is
/// already pinned; 6 lobster remain. Collection was progressing: six
/// named Takes plus four Eat, then a respawn Take on the first tile.
/// This observation does not rule out defects later in the cycle.
///
/// **Respawn (engine ticks — justification only):** `red_spiders_eggs.obj`
/// has no `respawnrate` (snape_grass sets 30). `ObjType.respawnrate = 100`
/// default "1-minute". `OBJ_TAKEITEM` on `EntityLifeCycle.RESPAWN` calls
/// `World.removeObj(obj, objType.respawnrate)`; `scaleByPlayerCount(100)`
/// with 1 player is 99 engine ticks. First Take 3117,9951 then the same
/// tile at FAIL; capture tick 141 matches ~100 engine ticks after first
/// take. Six distinct in-radius Takes before that respawn. Packed
/// `o48_155` obj list is not in this checkout; count is the measured 6.
///
/// **Provisional arm-to-deposit estimate:** remaining 6 lobster, eat at HP≤5 (lobster heal
/// 12 never fits maxHp 10). Measured 4 eats during the ~60s first-respawn
/// wait ≈ 15s/eat → ~90s more. Then Edgeville dungeon walk: field
/// 3120,9952 → ladder 3096,9868 Chebyshev 84 + surface ~25 + booth/deposit
/// ≈ 80s one-way. Watch already used ~60s. Arm-to-deposit ≈ 60+90+80 =
/// **~230s wall**. Measured dirty rate on this watch 150/60s ≈ 2.5 dirty/s
/// → **~575 dirties**. Use **600** for this bounded trial; estimates are not measured limits.
/// Return reverse dungeon ~80s × 2.5 ≈ 200 → **240** (same magnitude as
/// bone_burier / smithing bank-item). First-egg / empty / close / further
/// stay 150. Global gold 180s / 150 stay for every other script including
/// newt.
///
/// **Deadline:** seed+Start ~25s + deposit-watch wall ~240s + close ~10s +
/// return ~80s + further ~30s ≈ 385s. Use the existing **420s** pattern.
/// Script `walkTo` `timeoutMs` 180_000 per hop is unchanged. Diagnosis
/// remains provisional until root LIVE. No product seed, forced post-Start
/// bank, debug commands, or engine acceleration. Prior 1755 ladder stall
/// (empty-pack cell) is not baked into these numbers.
const HERBLORE_EGG_DEPOSIT_WATCH_TICKS: u32 = 600;
const HERBLORE_EGG_RETURN_WATCH_TICKS: u32 = 240;
const HERBLORE_EGG_DEADLINE: Duration = Duration::from_secs(420);

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

/// Janitor after combat `setstat` 1→40: click level-up continues until the
/// published continue/chat tree is gone. Uses [`Proof::NoActiveContinue`]
/// rather than [`Proof::ChatClosed`] alone — FireGiant baselines can keep
/// open-root continue widgets while `main_modal` is already -1.
fn drain_setstat_levelups_before_hostile_tele() -> Step {
    Step {
        name: "drain setstat level-up dialogs before the hostile-field teleport",
        kind: StepKind::DrainDialogs { choice: 1 },
        wait: Wait {
            arm: Proof::NoActiveContinue,
            budget_ticks: 60,
        },
    }
}

/// Insert the setstat level-up drain immediately before the hostile-field
/// teleport step (and therefore before Start/baseline).
fn insert_setstat_drain_before_hostile_tele(scenario: &mut Scenario) {
    let tele = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport step");
    scenario
        .steps
        .insert(tele, drain_setstat_levelups_before_hostile_tele());
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
    // Catalog food name comes from the selected loadout. The matching
    // fixture loadout is posted only at harness catalog Start.
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Memory food"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(22.0),
    },
    ScriptSettingInject {
        id: "bankAtFood",
        value: ScriptInjectValue::Num(3.0),
    },
];

/// Posted at panel harness catalog Start; not a Play/operator name reservation.
const THIEVER_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Memory food",
    carry: &[("Lobster", 1)],
}];

/// Durable prepare / run-prepared gates for Thiever (not script progress).
const THIEVER_FIXTURE_PREREQS: &[Proof] = &[
    Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 10,
    },
    Proof::Stat { id: 17, min: 50 },
    Proof::Stat { id: 3, min: 50 },
    Proof::Item {
        name: "Lobster",
        count: 10,
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
            fixture_prereqs: Some(THIEVER_FIXTURE_PREREQS),
            fixture_loadouts: Some(THIEVER_FIXTURE_LOADOUTS),
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

const RUNE_CHAINBODY_ID: i32 = 1113;
const CERT_RUNE_CHAINBODY_ID: i32 = 1114;
/// High Level Alchemy pays 60% of shop cost: floor(50000 * 0.6) = 30000.
const RUNE_CHAINBODY_HIGH_ALCH_COINS: i32 = 30_000;
/// Low Level Alchemy pays 40% of shop cost: floor(50000 * 0.4) = 20000.
const RUNE_CHAINBODY_LOW_ALCH_COINS: i32 = 20_000;
const LOW_ALCH_MAGIC_XP: i32 = 31;
const ATTACK_STAT: i32 = 0;

/// `spell=Low` on noted rune chainbodies, the `alcher-low-744-live` fixture:
/// Magic 25 at Varrock West with twelve chainbodies, 200 natures and a Staff
/// of fire banked. Nothing is worn and no outcome is seeded.
const ALCHER_LOW_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(10.0),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Low"),
    },
];

/// The `alcher-fire-battlestaff-live` fixture: High stays the default (no
/// `spell` inject) and the only fire staff banked is a Fire battlestaff.
const ALCHER_FIRE_BATTLESTAFF_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(8.0),
    },
];

/// The `alcher_low` scenario: Magic 25 at Varrock West, twelve rune
/// chainbodies / 200 natures / one Staff of fire banked, `spell=Low`, ten alchs
/// a trip. The core witness proves the Low cast arithmetic and the worn staff.
fn alcher_low_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: LOW_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 25 and the exact Low fixture stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 25");
                cheat(c, "givebank rune_chainbody 12");
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
    for (step_name, arm) in [
        (
            "confirm Magic 25 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 25,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
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
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no staff in the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact rune chainbody seed bank",
        Proof::BankItemId {
            id: RUNE_CHAINBODY_ID,
            count: 12,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact Staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no Fire battlestaff in the Low bank",
            Proof::BankItemIdAtMost {
                id: FIRE_BATTLESTAFF_ID,
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
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        (
            "watch the Staff of fire worn natively",
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
        ),
        ("watch Magic XP from a Low Level Alchemy cast", xp),
        (
            "watch the exact Low Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_LOW_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_low",
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
            script_settings_inject: Some(ALCHER_LOW_INJECT),
            terminal_shot: Some("alcher_low"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// The `alcher_fire_battlestaff` scenario: Magic 70 / Attack 40 at Varrock
/// West with eight chainbodies, 200 natures and exactly one Fire battlestaff
/// banked (no Staff of fire), default High, eight alchs a trip.
fn alcher_fire_battlestaff_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 70, Attack 40 and the exact alternative-staff stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 70");
                cheat(c, "setstat attack 40");
                cheat(c, "givebank fire_battlestaff 1");
                cheat(c, "givebank naturerune 200");
                cheat(c, "givebank rune_chainbody 8");
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
            "confirm Magic 70 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 70,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: ATTACK_STAT,
                min: 40,
            },
        ),
        (
            "confirm no seeded Fire battlestaff in the pack before Start",
            Proof::ItemIdAtMost {
                id: FIRE_BATTLESTAFF_ID,
                count: 0,
            },
        ),
        (
            "confirm the default Staff of fire is absent from the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
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
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact Fire battlestaff seed bank",
        Proof::BankItemId {
            id: FIRE_BATTLESTAFF_ID,
            count: 1,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact rune chainbody seed bank",
            Proof::BankItemId {
                id: RUNE_CHAINBODY_ID,
                count: 8,
            },
        ),
        (
            "acknowledge Staff of fire is absent from the alternative-staff bank",
            Proof::BankItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
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
            "watch the Fire battlestaff worn natively",
            Proof::EquipmentId {
                id: FIRE_BATTLESTAFF_ID,
            },
        ),
        (
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from a High Level Alchemy cast", xp),
        (
            "watch the exact High Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_fire_battlestaff",
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
            script_settings_inject: Some(ALCHER_FIRE_BATTLESTAFF_INJECT),
            terminal_shot: Some("alcher_fire_battlestaff"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Frozen `alcher-swarm-drain-live`: Magic 70, 20 rich + 8 poor, High default,
/// 20 alchs a trip. After the first native High cast, inject `~macro_event 1`
/// until a Swarm NPC targets the local player. Busy-player `please_finish`
/// is explicit rejection, not spawn. CoreWatch owns interruption/recovery.
const SWARM_MACRO_EVENT_CHEAT: &str = "~macro_event 1";
const SWARM_NPC_NAME: &str = "Swarm";
const SWARM_BUSY_REJECT: &str = "Please finish what you are doing first.";

fn swarm_macro_event_accepted(snap: &GameSnapshot) -> bool {
    Proof::NpcNameTargetingLocal {
        name: SWARM_NPC_NAME,
    }
    .check(snap, None)
}

fn swarm_busy_reject_seq(snap: &GameSnapshot) -> Option<i32> {
    snap.chat_lines().iter().find_map(|line| {
        line.text
            .contains(SWARM_BUSY_REJECT)
            .then_some(line.sequence)
    })
}

/// First attempt after the firstcast arms; later attempts only when chat
/// shows a newer busy rejection than the one consumed by the last send.
fn swarm_macro_event_should_send(
    snap: &GameSnapshot,
    ever_sent: bool,
    last_handled_reject_seq: i32,
) -> bool {
    if swarm_macro_event_accepted(snap) {
        return false;
    }
    if !ever_sent {
        return true;
    }
    swarm_busy_reject_seq(snap).is_some_and(|seq| seq > last_handled_reject_seq)
}

fn swarm_macro_event_inject(
    c: &mut Client,
    snap: &GameSnapshot,
    ever_sent: &AtomicBool,
    last_handled_reject_seq: &AtomicI32,
) -> bool {
    if !swarm_macro_event_should_send(
        snap,
        ever_sent.load(Ordering::Relaxed),
        last_handled_reject_seq.load(Ordering::Relaxed),
    ) {
        return true;
    }
    if !cheat(c, SWARM_MACRO_EVENT_CHEAT) {
        return false;
    }
    ever_sent.store(true, Ordering::Relaxed);
    if let Some(seq) = swarm_busy_reject_seq(snap) {
        last_handled_reject_seq.store(seq, Ordering::Relaxed);
    }
    true
}

const ALCHER_SWARM_DRAIN_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody", "yew_longbow"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(20.0),
    },
];

fn alcher_swarm_drain_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 70 and the exact swarm-drain stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 70");
                cheat(c, "givebank rune_chainbody 20");
                cheat(c, "givebank yew_longbow 8");
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
    for (step_name, arm) in [
        (
            "confirm Magic 70 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 70,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted Yew longbow before Start",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted Yew longbow before Start",
            Proof::ItemIdAtMost {
                id: YEW_LONGBOW_ID,
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
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no staff in the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact rune chainbody seed bank",
        Proof::BankItemId {
            id: RUNE_CHAINBODY_ID,
            count: 20,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Yew longbow seed bank",
            Proof::BankItemId {
                id: YEW_LONGBOW_ID,
                count: 8,
            },
        ),
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact Staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded note of the Yew longbow in bank",
            Proof::BankItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
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
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        (
            "watch the Staff of fire worn natively",
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
        ),
        ("watch Magic XP from a High Level Alchemy cast", xp),
        (
            "watch the exact High Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
        ),
        (
            "watch the first High cast consume a noted rune chainbody",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 19,
            },
        ),
        (
            "watch the first High cast consume a Nature rune",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 19,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let ever_sent = AtomicBool::new(false);
    let last_handled_reject_seq = AtomicI32::new(i32::MIN);
    steps.push(Step {
        name: "inject the upstream swarm macro_event after the first native High cast",
        kind: StepKind::Repeat {
            send: Box::new(move |c, snap| {
                swarm_macro_event_inject(c, snap, &ever_sent, &last_handled_reject_seq)
            }),
        },
        wait: Wait {
            arm: Proof::NpcNameTargetingLocal {
                name: SWARM_NPC_NAME,
            },
            budget_ticks: 50,
        },
    });
    Scenario {
        name: "alcher_swarm_drain",
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
            deadline: Duration::from_secs(420),
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_SWARM_DRAIN_INJECT),
            terminal_shot: Some("alcher_swarm_drain"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
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
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).close_modal(),
                    SendResult::Sent { .. } | SendResult::Refused { .. }
                )
            }),
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

const HERB_CLEANER_EMPTY_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf", "Marrentill"]),
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

/// Frozen reference fixture: one sub-full pack of guam, Marrentill selected
/// but absent, then the script's own eventual empty-bank Stop.
fn herb_cleaner_empty_bank_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore 20 and exactly 20 unidentified guam before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat herblore 20");
                cheat(c, "givebank unidentified_guam 20");
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
    for (step_name, arm) in [
        (
            "confirm Herblore 20 before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: 20,
            },
        ),
        (
            "confirm no unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge exactly 20 unidentified guam",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Marrentill is absent from the loaded seed bank",
        Proof::BankItemIdAtMost {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch script-created clean guam without requiring a full pack",
        Proof::ItemId {
            id: GUAM_LEAF_ID,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Herblore XP from post-Start guam cleaning",
        xp,
    ));
    Scenario {
        name: "herb_cleaner_empty_bank",
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
            deadline: Duration::from_secs(420),
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(HERB_CLEANER_EMPTY_INJECT),
            terminal_shot: Some("herb_cleaner_empty_bank"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
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
/// FlaxRunner meet tile. Same stand as the shared pair witness.
const FLAX_MEET: WorldTile = WorldTile {
    x: 2719,
    z: 3471,
    level: 0,
};
/// Duel Arena challenge-area seed. Same stand as the shared pair witness.
const DUEL_CHALLENGE: WorldTile = WorldTile {
    x: 3368,
    z: 3274,
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
/// Selected 289 `obj.pack`: silver_ore=442, silver_bar=2355.
const SILVER_ORE_ID: i32 = 442;
const COAL_ID: i32 = 453;
const FIRE_BATTLESTAFF_ID: i32 = 1393;
const BRONZE_BAR_ID: i32 = 2349;
const IRON_BAR_ID: i32 = 2351;
const STEEL_BAR_ID: i32 = 2353;
const SILVER_BAR_ID: i32 = 2355;
const SUPERHEAT_MAGIC: i32 = 43;
const BRONZE_SMITHING: i32 = 1;
/// SuperheaterLogic Silver recipe level.
const SILVER_SMITHING: i32 = 20;
const STEEL_SMITHING: i32 = 30;
/// SuperheaterLogic Mithril recipe level; 4 Coal per bar (5 bars / 27-slot trip).
const MITHRIL_SMITHING: i32 = 50;
/// Selected 289 `obj.pack`: mithril_ore=447, mithril_bar=2359.
const MITHRIL_ORE_ID: i32 = 447;
const MITHRIL_BAR_ID: i32 = 2359;
const SUPERHEATER_NATURES_SEED: i32 = 200;
const SUPERHEATER_ORE_SEED: i32 = 100;
const SUPERHEATER_COAL_SEED: i32 = 200;
/// SuperheaterLogic `NATURES_MIN` / one-slot nature stack + 27 ore slots.
const SUPERHEATER_NATURES_MIN: i32 = 28;
const SUPERHEATER_SINGLE_ORE_TRIP: i32 = 27;

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

const SUPERHEATER_SILVER_LOW_NATURES_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Silver"),
    },
    ScriptSettingInject {
        id: "natures",
        value: ScriptInjectValue::Num(28.0),
    },
];

const SUPERHEATER_MITHRIL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Mithril"),
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
    /// Single-ore 27-slot trip + minimum natures (28).
    Silver,
    /// Mithril ore + 4 Coal per bar (5 + 20 ores per trip).
    Mithril,
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

fn superheater_silver_low_natures_scenario() -> Scenario {
    superheater_variant(
        "superheater_silver_low_natures",
        SUPERHEATER_SILVER_LOW_NATURES_INJECT,
        SuperheaterRecipe::Silver,
        SuperheaterStaff::Fire,
    )
}

fn superheater_mithril_scenario() -> Scenario {
    superheater_variant(
        "superheater_mithril",
        SUPERHEATER_MITHRIL_INJECT,
        SuperheaterRecipe::Mithril,
        SuperheaterStaff::Fire,
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
    // Bronze/steel keep cumulative min-2 XP after restock. Silver uses a fresh
    // baseline so first-trip XP cannot satisfy resumed work alone.
    let (further_magic, further_smithing, terminal_proof) = if matches!(recipe, SuperheaterRecipe::Silver)
    {
        let fresh_magic = Proof::FreshStatXpGain {
            id: MAGIC_STAT,
            min: 1,
        };
        let fresh_smithing = Proof::FreshStatXpGain {
            id: SMITHING_STAT,
            min: 1,
        };
        (fresh_magic, fresh_smithing, fresh_smithing)
    } else {
        let further_magic = Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 2,
        };
        let further_smithing = Proof::StatXpGain {
            id: SMITHING_STAT,
            min: 2,
        };
        (further_magic, further_smithing, further_smithing)
    };
    // secondary_id is None for single-ore Silver (27 ore slots, no pair ore).
    let (bar_id, primary_id, secondary_id, smithing, staff_id, staff_alias) = match (recipe, staff)
    {
        (SuperheaterRecipe::Bronze, SuperheaterStaff::Fire) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            Some(TIN_ORE_ID),
            BRONZE_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::Fire) => (
            STEEL_BAR_ID,
            IRON_ORE_ID,
            Some(COAL_ID),
            STEEL_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Bronze, SuperheaterStaff::FireBattlestaff) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            Some(TIN_ORE_ID),
            BRONZE_SMITHING,
            FIRE_BATTLESTAFF_ID,
            "fire_battlestaff",
        ),
        (SuperheaterRecipe::Silver, SuperheaterStaff::Fire) => (
            SILVER_BAR_ID,
            SILVER_ORE_ID,
            None,
            SILVER_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Mithril, SuperheaterStaff::Fire) => (
            MITHRIL_BAR_ID,
            MITHRIL_ORE_ID,
            Some(COAL_ID),
            MITHRIL_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::FireBattlestaff)
        | (SuperheaterRecipe::Silver, SuperheaterStaff::FireBattlestaff)
        | (SuperheaterRecipe::Mithril, SuperheaterStaff::FireBattlestaff) => {
            unreachable!("recipe split is independent of the staff split")
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
                    SuperheaterRecipe::Silver => {
                        cheat(c, &format!("givebank silver_ore {SUPERHEATER_ORE_SEED}"));
                    }
                    SuperheaterRecipe::Mithril => {
                        cheat(c, &format!("givebank mithril_ore {SUPERHEATER_ORE_SEED}"));
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
    if let Some(secondary_id) = secondary_id {
        before_start.push((
            "confirm no seeded secondary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ));
    }
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
    match recipe {
        SuperheaterRecipe::Bronze => {
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Steel => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Silver => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Mithril => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
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
    if let Some(secondary_id) = secondary_id {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact secondary ore seed bank",
            Proof::BankItemId {
                id: secondary_id,
                count: match recipe {
                    SuperheaterRecipe::Bronze => SUPERHEATER_ORE_SEED,
                    SuperheaterRecipe::Steel | SuperheaterRecipe::Mithril => SUPERHEATER_COAL_SEED,
                    SuperheaterRecipe::Silver => unreachable!("silver has no secondary ore"),
                },
            },
        ));
    }
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
    // Nature ceiling is ordered after first cast XP/bar so empty-pack 0 cannot
    // satisfy the at-most check before the first withdrawal.
    let nature_after_cast = if matches!(recipe, SuperheaterRecipe::Silver) {
        SUPERHEATER_NATURES_MIN - 1
    } else {
        49
    };
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
                count: nature_after_cast,
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
    ];
    if matches!(recipe, SuperheaterRecipe::Silver) {
        watch.push((
            "watch a restock of the full single-ore trip",
            Proof::ItemId {
                id: primary_id,
                count: SUPERHEATER_SINGLE_ORE_TRIP,
            },
        ));
        watch.push((
            "watch natures topped to the minimum after restock",
            Proof::ItemId {
                id: NATURE_RUNE_ID,
                count: SUPERHEATER_NATURES_MIN,
            },
        ));
    } else {
        watch.push((
            "watch a restock of the exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ));
        if let Some(secondary_id) = secondary_id {
            watch.push((
                "watch a restock of the exact secondary ore",
                Proof::ItemId {
                    id: secondary_id,
                    count: match recipe {
                        SuperheaterRecipe::Bronze => 1,
                        SuperheaterRecipe::Steel => 2,
                        SuperheaterRecipe::Mithril => 4,
                        SuperheaterRecipe::Silver => unreachable!("silver has no secondary ore"),
                    },
                },
            ));
        }
    }
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
        proof: terminal_proof,
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
const PAIR_AIR_FIRST_LOAD: i32 = 25;
const PAIR_MULE_FIRST_LOAD: i32 = 27;

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
    steps.push(select_strength_combat_style_step());
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
    steps.push(select_strength_combat_style_step());
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
/// Canonical `FOOD_DEFAULT_COUNT` / `foodWithdraw` default. Carried at Start
/// so loot `needsRestock` is false (takeFood && foodCount < 1).
const HERBLORE_EGG_FOOD_CARRY: i32 = 10;
const HERBLORE_NEWT_COIN_SEED: i32 = 5000;
const EGG_FIELD: WorldTile = WorldTile {
    x: 3120,
    z: 9952,
    level: 0,
};
/// Safe native approach stand for the selected Edgeville booth. Headed shots
/// for both selected revision packs show this south-adjacent tile as
/// walkable, immediately next to booth 2213, without changing bank APIs.
const EDGEVILLE_BANK_APPROACH: WorldTile = WorldTile {
    x: 3096,
    z: 3494,
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
/// Stock-facing walkable adjacent for `DRAYNOR_BANK_BOOTH`. Capture
/// `2026-09-18T06-11-18` booth 2213@3091,3243 is shape 10 angle 1 force 0
/// 1×1; collision at 3092,3243 is open (0). Canonical `DRAYNOR_BANK`
/// 3093,3243 is Chebyshev 2 — `open_booth_at_matching` refuses Unreachable
/// and does not walk. Keep this seed stand separate from `DRAYNOR_BANK`.
const DRAYNOR_BANK_APPROACH: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
/// Selected 274/289 Draynor booth with native `Use-quickly` (id 2213). Capture
/// tile matches the open booth west of `DRAYNOR_BANK`; closed 2214/2215 are
/// not operable. Keep the walk stand separate from the booth identity.
const DRAYNOR_BANK_BOOTH: WorldTile = WorldTile {
    x: 3091,
    z: 3243,
    level: 0,
};
const DRAYNOR_BANK_BOOTH_ID: i32 = 2213;

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
const HERBLORE_EGGS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Red spiders' eggs"),
    },
    // scriptFood reads the selected loadout, not a food setting. Blank
    // loadout uses the operator's first saved carry (headed LIVE: Swordfish).
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Herblore food"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(HERBLORE_EGG_FOOD_CARRY as f64),
    },
];
const HERBLORE_EGGS_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Herblore food",
    carry: &[("Lobster", HERBLORE_EGG_FOOD_CARRY as u32)],
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
const GREEN_DRAGON_BASE_FOOD: i32 = 20;
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
/// The six verifiable Guard drops ArdyFighter lists in DEFAULT_LOOT
/// (`iron ore, steel arrow, body talisman, blood/chaos/nature rune`).
/// Its bank cell starts empty of this class.
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
/// Default source-script reset tile: inside the native visibility window but
/// outside the wake radius, so dormant `Rocks` can be observed before Start.
const ROCK_CRAB_SAFE_STAND: WorldTile = WorldTile {
    x: 2712,
    z: 3707,
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
const FIRE_GIANT_RAFT: WorldTile = WorldTile {
    x: 2510,
    z: 3493,
    level: 0,
};
const FIRE_GIANT_WASH: WorldTile = WorldTile {
    x: 2527,
    z: 3413,
    level: 0,
};
const FIRE_GIANT_BANK: WorldTile = WorldTile {
    x: 2616,
    z: 3332,
    level: 0,
};
const VARROCK_TELE_LAND: WorldTile = WorldTile {
    x: 3213,
    z: 3424,
    level: 0,
};
const ROCK_CRAB_BANK_RET: WorldTile = WorldTile {
    x: 2710,
    z: 3717,
    level: 0,
};
const SEERS_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3491,
    level: 0,
};
const FIRE_RUNE_ID: i32 = 554;
const VARROCK_TELE_MAGIC: i32 = 25;
const VARROCK_TELE_LAW: i32 = 3;
const VARROCK_TELE_AIR: i32 = 9;
const VARROCK_TELE_FIRE: i32 = 3;
const GREEN_DRAGON_BANK_RESTOCK: i32 = 20;
const FIRE_GIANT_BANK_RESTOCK: i32 = 20;

// Keep the seeded food independent of the operator's saved first loadout.
const CHAOS_DRUID_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Chaos Druid food",
    carry: &[("Lobster", 12)],
}];

const CHAOS_DRUID_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
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
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
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
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
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
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
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
const ROCK_CRAB_BANK_INJECT: &[ScriptSettingInject] = &[
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
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
];
const GREEN_DRAGON_TELE_INJECT: &[ScriptSettingInject] = &[
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
        value: ScriptInjectValue::Str("Teleport to Varrock"),
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
/// Bank cell: stall food plus the deposit class. `bankEveryItems=1` would
/// fire on a pre-Start listed Guard drop, so the class stays empty until a
/// kill feeds it.
const ARDY_FIGHTER_BANK_LOOT_EMPTY: &[i32] = &[
    CAKE_ID,
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];

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
                cheat(c, "setstat crafting 10");
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
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
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
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
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
                cheat(c, "setstat crafting 10");
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
                cheat(c, "setstat crafting 10");
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

/// Open one exact booth until the bank arm holds. Runner re-fires `Repeat`
/// every tick *before* checking the arm: re-clicking Use-quickly on an already
/// open loaded bank bumps the bank session and clears `bank_loaded` /
/// `bank_side`, so deposit then hard-fails. Skip the booth send once the
/// current session is open and loaded.
fn herblore_open_seed_bank_at(
    name: &'static str,
    arm: Proof,
    booth: WorldTile,
    booth_id: i32,
) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                if snapshot.bank_component_id() >= 0 && snapshot.bank_loaded() {
                    return true;
                }
                match Interactions::new(snapshot, c).open_booth_at(booth, booth_id) {
                    SendResult::Sent { .. } => true,
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { reason, .. } => {
                        eprintln!(
                            "[scenario] exact booth {booth_id}@{},{} send refused: {reason:?}",
                            booth.x, booth.z
                        );
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

fn herblore_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    herblore_open_seed_bank_at(
        name,
        arm,
        EDGEVILLE_BANK_BOOTH,
        EDGEVILLE_BANK_BOOTH_ID,
    )
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

fn herblore_newt_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Draynor booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: DRAYNOR_BANK_BOOTH_ID,
            x: DRAYNOR_BANK_BOOTH.x,
            z: DRAYNOR_BANK_BOOTH.z,
            level: DRAYNOR_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}

/// HerbloreSecondaries default Red spiders' eggs. Banked lobster via native
/// note seed + deposit (stock289 has no `givebank`). Fixture loadout pins
/// `scriptFood` to Lobster (no food setting; blank loadout uses the operator
/// first carry). Carry `FOOD_DEFAULT_COUNT` at the field so loot starts
/// instead of an empty-pack Edgeville restock. Ground Take 223, deposit,
/// empty product pack, close, return, further Take. Eggs are not given.
fn herblore_secondaries_scenario() -> Scenario {
    let field = EGG_FIELD;
    let bank_approach = EDGEVILLE_BANK_APPROACH;
    let eggs = Proof::ItemId {
        id: RED_SPIDERS_EGGS_ID,
        count: 1,
    };
    // stock289: 379=lobster (nonstackable), 380=cert_lobster (stackable note).
    let lobster = NativeSeed {
        unnoted_id: LOBSTER_ID,
        debug_alias: "lobster",
        note_alias: Some("cert_lobster"),
        quantity: HERBLORE_EGG_FOOD_SEED,
        note_id: Some(NOTED_LOBSTER_ID),
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked lobster and tele to the safe Edgeville booth approach before Start",
        bank_approach,
        vec![lobster],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact noted lobster seed in pack before deposit",
            Proof::ItemId {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the noted lobster seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_seed_bank_readiness());
    steps.push(herblore_open_seed_bank(
        "open and acknowledge the lobster seed bank",
        Proof::BankItemIdAtMost {
            id: LOBSTER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the noted lobster seed through the bank window",
        vec![lobster],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact lobster food seed bank",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the lobster food seed bank count",
            Proof::BankItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm the noted lobster seed was removed from pack",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no noted lobster remains in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted eggs in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
    // Carry canonical food so takeFood loot does not BankTrip before first Take.
    steps.push(Step {
        name: "tele to the Edgeville dungeon egg field with lobster food before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give lobster {HERBLORE_EGG_FOOD_CARRY}"));
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
        (
            "confirm no seeded noted lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm lobster food carry in pack before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
            },
        ),
        (
            "bound the lobster food carry in pack before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
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
        // Deposit arms on first ground egg while food/respawn still run.
        // Return is the reverse dungeon hop. See HERBLORE_EGG_* constant
        // docs (dirty increments, not 150×600ms). Other arms keep 150.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            HERBLORE_EGG_DEPOSIT_WATCH_TICKS
        } else if matches!(arm, Proof::ArrivedNear { .. }) {
            HERBLORE_EGG_RETURN_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm,
                budget_ticks,
            },
        });
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
            deadline: HERBLORE_EGG_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_EGGS_INJECT),
            fixture_loadouts: Some(HERBLORE_EGGS_FIXTURE_LOADOUTS),
            terminal_shot: Some("herblore_secondaries"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// HerbloreSecondaries Eye of newt shop branch. Banked coins via native
/// stackable seed + deposit (stock289 has no `givebank`), Betty stand.
/// Distinct from ground eggs. LIVE still waits on Shop.buy publication.
fn herblore_secondaries_newt_scenario() -> Scenario {
    let shop = BETTY_SHOP;
    let newt = Proof::ItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    };
    // stock289: 995=coins (stackable base; no certificate).
    let coins = NativeSeed {
        unnoted_id: COINS_ID,
        debug_alias: "coins",
        note_alias: None,
        quantity: HERBLORE_NEWT_COIN_SEED,
        note_id: None,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked coins at the Draynor booth approach before Start",
        DRAYNOR_BANK_APPROACH,
        vec![coins],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_newt_seed_bank_readiness());
    steps.push(herblore_open_seed_bank_at(
        "open and acknowledge the coin seed bank",
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        },
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
    ));
    steps.extend(native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![coins],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm the coin seed was removed from pack",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted newt in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
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
    /// Pre-Start wear step: the step's own name plus the item id its
    /// `Proof::EquipmentId` arm acknowledges. The label belongs to the item
    /// ("Dragonfire shield", "Adamant scimitar"), so it stays accurate when
    /// the helper is reused outside the shield cells.
    wear: Option<(&'static str, i32)>,
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    /// The native quest prerequisite completed before the stat/inventory
    /// reset and the hostile-field teleport (never a fabricated stage: the
    /// quest's own completion script runs and its journal row is
    /// acknowledged before Start).
    complete_quest: Option<NativeQuestPrereq>,
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
        wear,
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
    if let Some(prereq) = complete_quest {
        steps.extend(quest_prereq_steps(prereq));
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
    if let Some((label, id)) = wear {
        steps.push(wear_combat_item_step(label, id));
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
            fixture_loadouts: (card == "ChaosDruidKiller").then_some(CHAOS_DRUID_FIXTURE_LOADOUTS),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// A quest prerequisite the fixture completes through the **native** debug
/// quest-journal command `~quest` (`_test/scripts/cheats/cheat_quest.rs2`
/// `@debug_quests`): "Select Individual Quest." → the quest's row on the
/// paginated `p_choice5_header` list → "Complete.", which queues that
/// quest's own `[queue,<quest>_complete]` script. The path is targeted: the
/// required quest's completion is the only script queued, so no *other*
/// completion's rewards can arrive after Start (the demonstrated
/// `~completequests` leak: `QuestDone(Waterfall)` returned while ~60 queued
/// completions still owed stats/items/dialogs).
///
/// `dialog` is the quest's `quest_names_enum` label
/// (`content/scripts/general/configs/quest.enum`: index 34 `Lost City`,
/// index 50 `Waterfall Quest`) — the text the native list renders as
/// `<index>. <label>`; `journal` is the quest-tab row the native
/// `~send_quest_complete` helper paints green
/// (`content/scripts/player/interfaces/questlist.if` `[zanaris]`/`[waterfall]`,
/// `content/scripts/general/scripts/quests.rs2`), the acknowledgement the
/// fixture waits for.
///
/// The acknowledgement is the completion boundary itself, verified in clean
/// native content: `quest_zanaris.rs2` `[queue,zanaris_quest_complete]` sets
/// `%zanaris = ^zanaris_complete` (the var `levelrequire_zanaris_quest` gates
/// `opheld2 dragon_dagger` on, `levelrequire/scripts/levelrequire.rs2`) and
/// then calls `~send_quest_complete(questlist:zanaris, …)`; the FireGiant
/// counterpart `[queue,waterfall_quest_complete]`
/// (`quest_waterfall/scripts/quest_waterfall.rs2`) banks 2 diamonds, 2 gold
/// bars, 40 mithril seeds and 137,500 attack + strength XP before the same
/// helper. Every reward statement precedes the green, and `send_quest_complete`
/// is the last call in both scripts, so a fixture that waits for the green and
/// then resets stats/inventory cannot have quest rewards land after Start.
#[derive(Clone, Copy)]
struct NativeQuestPrereq {
    dialog: &'static str,
    journal: &'static str,
}

/// Waterfall Quest (`quest_names_enum` 50, journal row `[waterfall]`) — the
/// FireGiant dungeon entry's requirement.
const WATERFALL_QUEST_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Waterfall Quest",
    journal: "Waterfall Quest",
};

/// Lost City Of Zanaris (`quest_names_enum` 34, journal row `[zanaris]`):
/// `levelrequire_zanaris_quest_attack(60, last_slot)` gates
/// `opheld2 dragon_dagger` (`content/scripts/levelrequire/scripts/tier60.rs2`,
/// `content/scripts/levelrequire/scripts/levelrequire.rs2`).
const LOST_CITY_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Lost City",
    journal: "Lost City",
};

/// Shilo Village (`quest_names_enum` 43, journal row `[zombiequeen]` text
/// `Shilo Village`): `%zombiequeen >= ^zombiequeen_complete` (15) is the
/// wooden-gate membership (`quest_zombiequeen.rs2` `[oploc1,_shilo_woodengate]`).
const SHILO_VILLAGE_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Shilo Village",
    journal: "Shilo Village",
};

/// The native quest-journal command and the labels of its individual-quest
/// path: the first dialog's "Select Individual Quest." branch button, the
/// paginated list's "Next." button and the per-quest "Complete."
/// confirmation. The labels are the dialog's own resume-button texts, so
/// the machine presses a button by what it says, never by page arithmetic.
const QUEST_JOURNAL_CHEAT: &str = "~quest";
const QUEST_SELECT_INDIVIDUAL: &str = "Select Individual Quest.";
const QUEST_PAGE_NEXT: &str = "Next.";
const QUEST_COMPLETE_LABEL: &str = "Complete.";

/// The dialog this machine has already answered: the chat modal it was
/// rendered in and the option texts it carried. `chat.rs2` reuses one
/// interface per dialog shape (`multi4`, `multi5`, `multi3`) and
/// re-registers the *same* components as resume buttons for the question
/// that follows (`if_addresumebutton(multi5:com_5)`), and the engine
/// resumes a paused script with the clicked component
/// (`IfButtonHandler` → `p_pausebutton` → `last_com`). A press repeated
/// into the replaced dialog is therefore not inert — the page list's
/// `Next.` would advance a second page. One press per distinct dialog;
/// the step then waits for the next one, like the player it stands in for.
#[derive(Default)]
struct QuestJournalState {
    answered: Option<(i32, String)>,
}

/// One tick of the native quest-journal dialog machine: continue a
/// `BUTTON_CONTINUE` chat IF (the completion's level-ups), else answer the
/// option dialog the individual-quest path waits on (Select Individual
/// Quest. → the required quest's page row → Complete.), else close the
/// modal a completion opened (the quest scroll `send_quest_complete` opens
/// just before it paints the row green). Returns `false` when an option
/// dialog cannot be placed: the preparation step fails explicitly instead
/// of guessing a button.
fn answer_quest_journal_dialogs(
    client: &mut Client,
    snapshot: &GameSnapshot,
    prereq: NativeQuestPrereq,
    state: &mut QuestJournalState,
) -> bool {
    if snapshot.chat_continue_component_id() != -1 {
        let mut ix = Interactions::new(snapshot, client);
        return matches!(
            ix.continue_dialog(),
            SendResult::Sent { .. } | SendResult::Refused { .. }
        );
    }
    if !snapshot.chat_options().is_empty() {
        let identity = (
            snapshot.modals().chat,
            snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if state.answered.as_ref() == Some(&identity) {
            // Already answered this dialog; the next one has not landed.
            return true;
        }
        let choose = |wanted: &dyn Fn(&str) -> bool| {
            snapshot
                .chat_options()
                .iter()
                .position(|option| wanted(option.text.as_str()))
        };
        let choice = choose(&|text| text == QUEST_SELECT_INDIVIDUAL)
            .or_else(|| choose(&|text| text == QUEST_COMPLETE_LABEL))
            .or_else(|| choose(&|text| text.ends_with(prereq.dialog)))
            .or_else(|| choose(&|text| text == QUEST_PAGE_NEXT));
        let Some(choice) = choice else {
            return false;
        };
        let mut ix = Interactions::new(snapshot, client);
        return match ix.answer_choice(choice as i32 + 1) {
            SendResult::Sent { .. } => {
                state.answered = Some(identity);
                true
            }
            // Nothing pressable in that snapshot: retry on the next tick.
            SendResult::Refused { .. } => true,
        };
    }
    let m = snapshot.modals();
    if m.main == -1 && m.side == -1 && m.chat == -1 && m.tutorial == -1 {
        return true;
    }
    let mut ix = Interactions::new(snapshot, client);
    matches!(
        ix.close_modal(),
        SendResult::Sent { .. } | SendResult::Refused { .. }
    )
}

/// Pre-Start native quest prerequisite: send `~quest`, answer its
/// individual-quest dialogs until the required quest's "Complete." queued
/// that quest's own completion script, then drain the completion's
/// scroll/level-up dialogs until the native helper paints the quest's
/// journal row green. That green row is the completion boundary: the stat
/// and inventory reset, the prepared-gear acknowledgements and the
/// hostile-field teleport all follow it, so no quest reward can arrive
/// after Start. The step's tick budget bounds a preparation that never
/// acknowledges — the run fails before Start instead of starting without
/// the prerequisite.
fn quest_prereq_steps(prereq: NativeQuestPrereq) -> [Step; 2] {
    // The machine is re-entered once per tick by the runner's `Repeat`
    // arm; its "already answered this dialog" state is behind a mutex so
    // the step closure stays `Fn + Send + Sync` (the `StepKind` bound).
    let state = std::sync::Mutex::new(QuestJournalState::default());
    [
        Step {
            name: "open the native quest-journal prerequisite dialog before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, QUEST_JOURNAL_CHEAT);
                    true
                }),
            },
            wait: Wait {
                arm: Proof::ChatChoice,
                budget_ticks: 20,
            },
        },
        Step {
            name: "answer the quest-journal dialogs until the required quest is acknowledged",
            kind: StepKind::Repeat {
                send: Box::new(move |c, snapshot| {
                    let Ok(mut state) = state.lock() else {
                        return false;
                    };
                    answer_quest_journal_dialogs(c, snapshot, prereq, &mut state)
                }),
            },
            wait: Wait {
                arm: Proof::QuestDone {
                    name: prereq.journal,
                },
                budget_ticks: 600,
            },
        },
    ]
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

/// Select and acknowledge the native aggressive melee style after wielding.
fn is_aggressive_combat_style(label: &str) -> bool {
    label.to_ascii_lowercase().contains("aggressive")
}

fn select_strength_combat_style_step() -> Step {
    Step {
        name: "select and acknowledge native Strength combat style before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                let Some(root) = snapshot
                    .side_tabs()
                    .iter()
                    .find(|tab| tab.index == 0 && tab.available)
                    .map(|tab| tab.root_component_id)
                else {
                    return false;
                };
                let Some(style) =
                    api::query::widget_search::combat_style_labels(snapshot, root, 43)
                        .into_iter()
                        .find(|style| style.mode == 1 && is_aggressive_combat_style(&style.label))
                else {
                    return false;
                };
                let ctx = ReadContext::new(snapshot);
                let Some(widget) = ctx.component(style.component_id) else {
                    return false;
                };
                matches!(
                    Interactions::new(snapshot, c).press(widget),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::VarpExact { id: 43, value: 1 },
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
        wear: None,
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
        wear: None,
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
        wear: None,
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
        wear: None,
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
        wear: None,
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
        wear: None,
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
/// Catalog requires native Rocks activation into Rock Crab, and the melee
/// fixture arrives already wearing the scoped scimitar: the frozen card's own
/// `GearEquip` refuses a carried melee fixture, so a packed 1331 would fight
/// unarmed (the native pre-Start wear is the proof). Banking is not this cell.
/// SolveClue stays injected off.
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
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
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
        food_count: GREEN_DRAGON_BASE_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
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
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_SPECIAL_INJECT,
        complete_quest: Some(LOST_CITY_PREREQ),
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
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
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
/// rope 954 use existing allowed preparation. Frozen `GearEquip` refuses a
/// carried melee fixture, so the cell wears 1331 before the hostile tele;
/// setstat 1→40 queues level-up continues that must drain before Start.
fn fire_giant_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
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
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: FIRE_GIANT_LOOT_EMPTY,
        inject: FIRE_GIANT_INJECT,
        complete_quest: Some(WATERFALL_QUEST_PREREQ),
        thieving: 0,
        agility: 0,
    });
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
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
        wear: None,
        loot_empty: ARDY_FIGHTER_LOOT_EMPTY,
        inject: ARDY_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 5,
        agility: 0,
    })
}

/// `banking=Auto` on AutoFighter: BankRun walks to the nearest bank from the
/// anchor, deposits everything its keep-list does not hold and restocks food.
/// Custom `loot=Bones` uses the Guard's guaranteed drop with burial disabled.
/// `bankAtLootSlots=1` ends the trip on that script-looted drop, without
/// depending on a random secondary drop or seeding deposit-class items.
const AUTO_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_BANK_RESTOCK as f64),
    },
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
        value: ScriptInjectValue::StrList(&["Bones"]),
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
/// has to draw from, the scoped weapon itself (seeded, acknowledged carried,
/// then worn — never a card stash: the card's own deposit must not be able to
/// stash it), and the bank/return watch chain after Start.
#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario(
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
                cheat(c, &format!("give {weapon_alias} 1"));
                // Zero-count food is a declared empty baseline, never a
                // `give`: the native `::give` handler clamps to at least
                // one (`Math.max(1, …)`), so `give cake 0` seeded the Cake
                // the ardy_fighter_bank baseline is supposed to lack.
                if food_count > 0 {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
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
    // ChaosDruid's seeded food shortfall deliberately starts with banking.
    // Its ordered watches and CoreWatch require combat after the return;
    // waiting for incidental auto-retaliation XP here gates the wrong phase.
    if name != "chaos_druid_bank" {
        steps.push(bank_fletcher_watch(
            "watch Strength XP from the selected melee style after Start",
            xp,
        ));
    }
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
            fixture_loadouts: (card == "ChaosDruidKiller").then_some(CHAOS_DRUID_FIXTURE_LOADOUTS),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Custom Bones loot with burial disabled exercises AutoFighter's BankRun
/// after a guaranteed Guard drop. No Bones are seeded: the script must loot
/// them, deposit at East Ardougne, restock ten Trout, close, return and fight.
fn auto_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "auto_fighter_bank",
        "AutoFighter",
        ARDOUGNE_GUARD,
        8,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        &[BONES_ID],
        AUTO_FIGHTER_BANK_INJECT,
        0,
        "trout",
        20,
        &[
            (
                "watch a Guard drop enter a fresh Ardougne East bank",
                Proof::BankItemIdAny {
                    ids: &[BONES_ID],
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
        "adamant_scimitar",
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
        "adamant_scimitar",
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
        "adamant_scimitar",
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
                    // Match the script's WalkNear return radius; broad proximity
                    // can pass while the final hop is still cancelled.
                    radius: 4,
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
/// Nothing in the deposit class is prepared: `bankEveryItems=1` would
/// otherwise treat a pre-Start listed item as the trip end.
fn ardy_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "ardy_fighter_bank",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        0,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        ARDY_FIGHTER_BANK_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch a Guard drop enter a fresh East Ardougne bank",
                Proof::BankItemIdAny {
                    ids: &GUARD_DROP_IDS,
                    count: 1,
                },
            ),
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

const ROCK_CRAB_BANK_DEPOSIT: [i32; 2] = [UNCUT_SAPPHIRE_ID, CASKET_ID];
const GREEN_DRAGON_BANK_DEPOSIT: [i32; 2] = [DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID];

/// FireGiant's Waterfall Quest prerequisite via the native individual-quest
/// path, ahead of the stat/inventory reset — the same targeted completion
/// `combat_core_scenario` runs for its `complete_quest` cells.
fn insert_waterfall_quest(scenario: &mut Scenario) {
    let prepare = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("prepare melee stats"))
        .expect("combat bank/core prepares stats");
    scenario
        .steps
        .splice(prepare..prepare, quest_prereq_steps(WATERFALL_QUEST_PREREQ));
}

fn wear_shield_before_hostile_teleport(scenario: &mut Scenario) {
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        wear_combat_item_step(
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        ),
    );
}

fn acknowledge_dormant_rocks_before_start(scenario: &mut Scenario) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat bank has a Start step");
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
}

/// RockCrab PeriodicBank `Loot count` at Seers. `bankEveryItems=1` ends the
/// trip on the first listed drop (uncut sapphire 1623 or casket 405). The
/// PeriodicBank task deposits matching loot and returns to `currentSpot()`;
/// it does not restock food, so this cell seeds no bank stock at all: the
/// frozen card's `BankRun` (the food-gone withdraw, a different trip) is the
/// only reader of a bank food window and eight lobster outlast the cell.
fn rock_crab_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "rock_crab_bank",
        "RockCrab",
        ROCK_CRAB_SAFE_STAND,
        2,
        "lobster",
        LOBSTER_ID,
        ROCK_CRAB_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        ROCK_CRAB_LOOT_EMPTY,
        ROCK_CRAB_BANK_INJECT,
        0,
        "",
        0,
        &[
            (
                "watch listed RockCrab loot enter a fresh Seers bank",
                Proof::BankItemIdAny {
                    ids: &ROCK_CRAB_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the periodic bank at the Seers booth",
                Proof::ArrivedNear {
                    x: SEERS_BANK.x,
                    z: SEERS_BANK.z,
                    level: SEERS_BANK.level,
                    radius: 6,
                },
            ),
            ("watch the periodic bank close", Proof::BankClosed),
            (
                "watch return to the nearest RockCrab spot after banking",
                Proof::ArrivedNear {
                    x: ROCK_CRAB_BANK_RET.x,
                    z: ROCK_CRAB_BANK_RET.z,
                    level: ROCK_CRAB_BANK_RET.level,
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
    );
    acknowledge_dormant_rocks_before_start(&mut scenario);
    scenario
}

/// GreenDragon BankRun to Edgeville: food-gone / pack-full trip deposits
/// bones or hide, restocks lobster to `foodWithdraw` 20, and walks back
/// past the wilderness ditch.
fn green_dragon_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_bank",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch dragon bones or hide enter a fresh Edgeville bank",
                Proof::BankItemIdAny {
                    ids: &GREEN_DRAGON_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            ("watch GreenDragon close its bank", Proof::BankClosed),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    scenario
}

/// `escape=Teleport to Varrock`: Magic XP and a Varrock land, then the
/// Edgeville booth restock and a return past the ditch. A south-walk flee
/// without the teleport fails this cell.
fn green_dragon_tele_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_tele",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("lawrune", LAW_RUNE_ID, VARROCK_TELE_LAW),
            ("airrune", AIR_RUNE_ID, VARROCK_TELE_AIR),
            ("firerune", FIRE_RUNE_ID, VARROCK_TELE_FIRE),
        ],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_TELE_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch Magic XP from the Varrock teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the Varrock teleport land, not a queued button",
                Proof::ArrivedNear {
                    x: VARROCK_TELE_LAND.x,
                    z: VARROCK_TELE_LAND.z,
                    level: VARROCK_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            (
                "watch GreenDragon close its bank after the teleport",
                Proof::BankClosed,
            ),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [Step {
            name: "prepare and acknowledge Magic 25 for Varrock teleport before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {VARROCK_TELE_MAGIC}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: VARROCK_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        }],
    );
    scenario
}

/// FireGiant `EnterDungeon` from the raft. Start is not already z>=9000; the
/// frozen script has to board the raft, rope the rock and tree, open the
/// ledge door, then fight. Same frozen `GearEquip` melee refuse as the core
/// cell: wear 1331 and drain setstat level-ups before the raft teleport.
fn fire_giant_approach_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "fire_giant_approach",
        card: "FireGiant",
        tele: FIRE_GIANT_RAFT,
        radius: 5,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: FIRE_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: FIRE_GIANT_LOOT_EMPTY,
        inject: FIRE_GIANT_INJECT,
        complete_quest: Some(WATERFALL_QUEST_PREREQ),
        thieving: 0,
        agility: 0,
    });
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.insert(
        start + 1,
        bank_fletcher_watch(
            "watch the script enter the Waterfall Dungeon after Start",
            Proof::ArrivedNear {
                x: FIRE_GIANT_ROOM.x,
                z: FIRE_GIANT_ROOM.z,
                level: FIRE_GIANT_ROOM.level,
                radius: 16,
            },
        ),
    );
    scenario
}

/// FireGiant barrel exit then Ardougne West restock and re-entry. The in-room
/// core leaves this trip unqualified.
fn fire_giant_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "fire_giant_bank",
        "FireGiant",
        FIRE_GIANT_ROOM,
        10,
        "lobster",
        LOBSTER_ID,
        FIRE_GIANT_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        FIRE_GIANT_LOOT_EMPTY,
        FIRE_GIANT_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch the barrel wash-up before the bank walk",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_WASH.x,
                    z: FIRE_GIANT_WASH.z,
                    level: FIRE_GIANT_WASH.level,
                    radius: 6,
                },
            ),
            (
                "watch the trip's Big bones enter a fresh Ardougne West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the Ardougne West booth the barrel trip walks to",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_BANK.x,
                    z: FIRE_GIANT_BANK.z,
                    level: FIRE_GIANT_BANK.level,
                    radius: 6,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch FireGiant close its bank", Proof::BankClosed),
            (
                "watch re-entry to the fire-giant room after banking",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_ROOM.x,
                    z: FIRE_GIANT_ROOM.z,
                    level: FIRE_GIANT_ROOM.level,
                    radius: 10,
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
    );
    insert_waterfall_quest(&mut scenario);
    scenario
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

const FIREMAKING_STAT: i32 = 11;
const STAFF_OF_AIR_ID: i32 = 1381;
const STAFF_OF_WATER_ID: i32 = 1383;
const HAMMER_ID: i32 = 2347;
const BRONZE_BAR_CERT_ID: i32 = 2350;
/// Selected 289 `obj.pack`: `cert_steel_bar` = 2354 (unnoted steel bar 2353).
const STEEL_BAR_CERT_ID: i32 = 2354;
/// Selected 289 `obj.pack`: mithril_bar=2359, cert_mithril_bar=2360.
const MITHRIL_BAR_CERT_ID: i32 = 2360;
const BRONZE_DAGGER_ID: i32 = 1205;
/// Mithril dagger product id (stock smithing dbrow lvl 50).
const MITHRIL_DAGGER_ID: i32 = 1209;
const BRONZE_PLATEBODY_ID: i32 = 1117;
/// Selected 289 nails (steel-only anvil product, stackable, out 2/bar).
const STEEL_NAILS_ID: i32 = 1539;
/// Nails recipe levelrequired (steel bar tier is 30; product gate is 34).
const STEEL_NAILS_SMITHING: i32 = 34;
/// Stock nails `product_amount` — stack output must prove ≥ this count.
const STEEL_NAILS_OUTPUT: i32 = 2;
const NEEDLE_ID: i32 = 1733;
const THREAD_ID: i32 = 1734;
const LEATHER_GLOVES_ID: i32 = 1059;
const LEATHER_CHAPS_ID: i32 = 1095;
const HARDLEATHER_BODY_ID: i32 = 1131;
const DRAGONHIDE_BODY_ID: i32 = 1135;
const DRAGONHIDE_CHAPS_ID: i32 = 1099;
const LEATHER_CERT_ID: i32 = 1742;
const HARD_LEATHER_CERT_ID: i32 = 1744;
/// Green dragon leather (selected289 `dragon_leather` / alias `dragon_leather`).
const GREEN_DRAGON_LEATHER_ID: i32 = 1745;
const GREEN_DRAGON_LEATHER_CERT_ID: i32 = 1746;
/// Enough banked leather for two 26-slot trips, plus four pieces.
const LEATHER_CRAFTER_TWO_TRIP_SEED: i32 = 56;
/// Banked coins for missing-thread `fundThread` (default threadPerTrip 100 ×
/// THREAD_MAX_PRICE 3 = 300 needed; 1000 leaves headroom).
const LEATHER_THREAD_SHOP_COIN_SEED: i32 = 1000;
/// Canonical fundThread withdrawal ceiling: threadPerTrip default 100 × max price 3.
const LEATHER_THREAD_SHOP_FUND: i32 = 300;
const LOGS_CERT_ID: i32 = 1512;
const OAK_LOGS_CERT_ID: i32 = 1522;
const OAK_LOGS_ID: i32 = 1521;
const TINDERBOX_ID: i32 = 590;

#[derive(Clone, Copy)]
struct NativeSeed {
    unnoted_id: i32,
    debug_alias: &'static str,
    /// Alias of the certificate object when this seed is bulk-noted.
    note_alias: Option<&'static str>,
    quantity: i32,
    note_id: Option<i32>,
}

fn native_seed_definition_valid(objs: &[client::config::ObjType], seed: NativeSeed) -> bool {
    let Some(base) = objs.get(seed.unnoted_id as usize) else {
        return false;
    };
    if base.id != seed.unnoted_id || base.certlink != -1 {
        return false;
    }
    if let Some(note_id) = seed.note_id {
        let Some(note) = objs.get(note_id as usize) else {
            return false;
        };
        note.id == note_id
            && note.stackable
            && note.certlink == seed.unnoted_id
            && note.certtemplate >= 0
    } else {
        base.stackable || seed.quantity <= 1
    }
}

fn native_bank_seed(
    name: &'static str,
    bank: WorldTile,
    seeds: Vec<NativeSeed>,
    skill: &'static str,
    stat: i32,
) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                // Resolve every fixture against the loaded cache before
                // sending a command. A missing or contradictory certificate
                // definition must fail closed, never become a name-only seed.
                for seed in &seeds {
                    if !native_seed_definition_valid(&c.cache.objs, *seed) {
                        return false;
                    }
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat {skill} {stat}"));
                for seed in &seeds {
                    let alias = seed.note_alias.unwrap_or(seed.debug_alias);
                    cheat(c, &format!("give {alias} {}", seed.quantity));
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
                radius: 8,
            },
            budget_ticks: 200,
        },
    }
}

fn native_bank_deposit(name: &'static str, seeds: Vec<NativeSeed>) -> Vec<Step> {
    seeds
        .into_iter()
        .map(|seed| Step {
            name,
            kind: StepKind::Repeat {
                send: Box::new(move |c, snapshot| {
                    let inv_id = seed.note_id.unwrap_or(seed.unnoted_id);
                    if (Proof::BankItemId {
                        id: seed.unnoted_id,
                        count: seed.quantity,
                    })
                    .check(snapshot, None)
                        && (Proof::ItemIdAtMost {
                            id: inv_id,
                            count: 0,
                        })
                        .check(snapshot, None)
                    {
                        return true;
                    }
                    // Open bank UI without a current full is not ready:
                    // bank_side is empty and deposit would hard-fail the
                    // Repeat send. Soft-wait on existing session signals;
                    // never invent rows or accept a closed/stale bank.
                    if snapshot.bank_component_id() >= 0 && !snapshot.bank_loaded() {
                        return true;
                    }
                    let mut ix = Interactions::new(snapshot, c);
                    snapshot
                        .bank_side()
                        .into_iter()
                        .filter(|item| {
                            // Bank-side rows are the pack: bulk fixtures sit
                            // there as certificates, not as unnoted bases.
                            item.def.id == inv_id
                                && match seed.note_id {
                                    Some(_) => {
                                        item.def.noted
                                            && item.def.certificate_link == seed.unnoted_id
                                    }
                                    None => !item.def.noted,
                                }
                        })
                        .filter_map(|item| bank_deposit_all_op(&item.actions).map(|op| (item, op)))
                        .any(|(item, op)| {
                            matches!(
                                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                                SendResult::Sent { .. }
                            )
                        })
                }),
            },
            wait: Wait {
                arm: Proof::BankItemId {
                    id: seed.unnoted_id,
                    count: seed.quantity,
                },
                budget_ticks: 200,
            },
        })
        .collect()
}
const LUMBRIDGE_BANK: WorldTile = WorldTile {
    x: 3092,
    z: 3245,
    level: 0,
};
const FALADOR_TELE_LAND: WorldTile = WorldTile {
    x: 2965,
    z: 3378,
    level: 0,
};
const AEMAD_STAND: WorldTile = WorldTile {
    x: 2613,
    z: 3294,
    level: 0,
};
const AUBURY_STAND: WorldTile = WorldTile {
    x: 3253,
    z: 3401,
    level: 0,
};
/// Frozen shopPresets Lowe stand (Varrock Archery Emporium).
const LOWE_STAND: WorldTile = WorldTile {
    x: 3231,
    z: 3421,
    level: 0,
};
/// Frozen shopPresets Hickton stand (Catherby Archery Emporium).
const HICKTON_STAND: WorldTile = WorldTile {
    x: 2821,
    z: 3442,
    level: 0,
};
/// Frozen shopPresets Harry stand (Catherby Fishing Shop).
const HARRY_STAND: WorldTile = WorldTile {
    x: 2833,
    z: 3443,
    level: 0,
};
/// Frozen shopPresets Betty stand (Port Sarim Magic Emporium). Distinct from
/// herblore `BETTY_SHOP` 3012,3259 — ShopBuyout pin is shopPresets 3012,3258.
const BETTY_STAND: WorldTile = WorldTile {
    x: 3012,
    z: 3258,
    level: 0,
};
/// Frozen shopPresets Gerrant stand (Port Sarim Fishy Business).
const GERRANT_STAND: WorldTile = WorldTile {
    x: 3013,
    z: 3224,
    level: 0,
};
/// Frozen shopPresets Bob stand (Lumbridge Axe Shop).
const BOB_STAND: WorldTile = WorldTile {
    x: 3231,
    z: 3203,
    level: 0,
};
/// Frozen shopPresets Nurmof stand (Dwarven Mine pickaxe shop).
const NURMOF_STAND: WorldTile = WorldTile {
    x: 2997,
    z: 9844,
    level: 0,
};
/// Frozen shopPresets Magic Store owner stand (Yanille Wizards' Guild, floor 1).
const MAGIC_STORE_STAND: WorldTile = WorldTile {
    x: 2594,
    z: 3090,
    level: 1,
};
/// Frozen shopPresets Lundail stand (Mage Arena cellar rune shop).
const LUNDAIL_STAND: WorldTile = WorldTile {
    x: 2535,
    z: 4719,
    level: 0,
};
/// Frozen shopPresets Gundai bankStand (Mage Arena cellar). No booth;
/// `magearena_banker` publishes Talk-to only (`mage_arena.npc`).
const GUNDAI_BANK_APPROACH: WorldTile = WorldTile {
    x: 2533,
    z: 4714,
    level: 0,
};
/// Frozen shopPresets Fernahei stand (Shilo fishing hut).
const FERNAHEI_STAND: WorldTile = WorldTile {
    x: 2870,
    z: 2971,
    level: 0,
};
/// Frozen shopPresets Shilo bankStand. No booth; `shilobanker` display
/// name is `Banker` (`banker.npc`).
const SHILO_BANK_APPROACH: WorldTile = WorldTile {
    x: 2852,
    z: 2954,
    level: 0,
};
/// Stock-facing walkable adjacent for Yanille open booth 2213@2614,3092
/// (map m40_48 local 54,20). Preset bankStand 2613,3092 is Chebyshev 1 west.
const YANILLE_BANK_APPROACH: WorldTile = WorldTile {
    x: 2613,
    z: 3092,
    level: 0,
};
const YANILLE_BANK_BOOTH: WorldTile = WorldTile {
    x: 2614,
    z: 3092,
    level: 0,
};
const YANILLE_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Ardougne East open booth 2213@2656,3283
/// (map m41_51 local 32,19). Preset bankStand 2655,3283 is Chebyshev 1 west;
/// closed 2215@2656,3280 is not operable. Keep the seed stand off the booth.
const AEMAD_BANK_APPROACH: WorldTile = WorldTile {
    x: 2655,
    z: 3283,
    level: 0,
};
const AEMAD_BANK_BOOTH: WorldTile = WorldTile {
    x: 2656,
    z: 3283,
    level: 0,
};
const AEMAD_BANK_BOOTH_ID: i32 = 2213;
/// Catalog Varrock East stand (content.rs) north of open booth 2213@3253,3419
/// (m50_53 local 53,27). Closed 2215@3251/3255,3419 are not operable.
const VARROCK_EAST_BANK_APPROACH: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};
const VARROCK_EAST_BANK_BOOTH: WorldTile = WorldTile {
    x: 3253,
    z: 3419,
    level: 0,
};
const VARROCK_EAST_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Catherby open booth 2213@2809,3442
/// (map m43_53 local 57,50 shape 10 angle 2). Preset bankStand 2809,3441 is
/// Chebyshev 1 south; closed 2215@2806/2808/2812,3442 are not operable.
/// Reuses the existing cook CATHERBY_BANK stand identity.
const CATHERBY_BANK_APPROACH: WorldTile = CATHERBY_BANK;
const CATHERBY_BANK_BOOTH: WorldTile = WorldTile {
    x: 2809,
    z: 3442,
    level: 0,
};
const CATHERBY_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Falador West open booth 2213@2946,3367
/// (vial_filler/climbing_boots booth identity). Preset bankStand 2946,3369 is
/// Chebyshev 2 north of the booth; approach reuses FALADOR_WEST_BANK 2946,3368
/// (Chebyshev 1). Keep the seed stand off the booth tile.
const FALADOR_WEST_BANK_APPROACH: WorldTile = FALADOR_WEST_BANK;
const FALADOR_WEST_BANK_BOOTH: WorldTile = FALADOR_WEST_BOOTH;
const FALADOR_WEST_BANK_BOOTH_ID: i32 = 2213;
const VARROCK_ANVIL: WorldTile = WorldTile {
    x: 3188,
    z: 3425,
    level: 0,
};

/// ShopBuyout coin seed: enough banked gp for two+ trips under the injected
/// budgets without product/ballast seed. Deposited through the ordinary booth.
const SHOP_BUYOUT_COIN_SEED: i32 = 20_000;
/// Aemad nonstackable representative: shopdb adventurershop `vial_water`
/// baseline 500 cost 2 sell 1300 → ~2gp/unit at stock; 28-slot pack fill is
/// the natural bank trigger (needSpaceFor). perTrip covers one pack + margin;
/// budget > perTrip leaves sessionSpent headroom for a resumed buy after
/// deposit (budget==perTrip can Stop on budget spent and kill resume).
const SHOP_BUYOUT_AEMAD_PER_TRIP_GP: f64 = 200.0;
const SHOP_BUYOUT_AEMAD_BUDGET_GP: f64 = 600.0;
/// Aubury stackable representative: shopdb runeshop `firerune` baseline 2000
/// cost 4 sell 1000 → 4gp/unit. Pack fill does not bank stackables that
/// already hold a stack; natural bank is coins<100 after a real buy (or
/// budget spent). perTrip 500 spends down under 100; budget 1500 keeps
/// remaining budget for a second withdraw+buy.
const SHOP_BUYOUT_AUBURY_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_AUBURY_BUDGET_GP: f64 = 1500.0;
/// Lowe/Hickton stackable bronze arrows: cost 1 sell 1000 → 1gp/unit at
/// baseline (Lowe stock 2000, Hickton 1000). Same stackable bank trigger as
/// Aubury (coins<100); perTrip 500 exhausts under 100 with stock left.
const SHOP_BUYOUT_LOWE_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_LOWE_BUDGET_GP: f64 = 1500.0;
const SHOP_BUYOUT_HICKTON_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_HICKTON_BUDGET_GP: f64 = 1500.0;
/// Harry stackable fishing bait: cost 3 sell 1000 → 3gp/unit; baseline 1200.
/// perTrip 500 buys ~166 units (498gp) and leaves coins under 100.
const SHOP_BUYOUT_HARRY_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_HARRY_BUDGET_GP: f64 = 1500.0;
/// Betty stackable fire rune: magicshop firerune baseline 1000 cost 4 sell 1000
/// → 4gp/unit (same unit math as Aubury). perTrip 500 / budget 1500 for coins<100
/// bank + resume headroom. Stock 1000 leaves remainder after one trip.
const SHOP_BUYOUT_BETTY_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_BETTY_BUDGET_GP: f64 = 1500.0;
/// Gerrant stackable feather: fishingshop feather baseline 1000 cost 2 sell 1000
/// → 2gp/unit. perTrip 500 buys ~250 units and leaves coins under 100.
const SHOP_BUYOUT_GERRANT_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_GERRANT_BUDGET_GP: f64 = 1500.0;
/// Bob nonstackable steel axe: axeshop sell 1000 delta 20, baseline 3 cost 200.
/// Frozen ShopBuyout banks when `coins < 100` after a real buy (BuyoutPass.ts).
/// Stock-sensitive unit_price at full shelf: stock3→200, stock2→204 (208 unaffordable on 3rd).
/// perTrip 500 → buyoutPlan 200+204=404 spent → 96 coins (<100 bank); 1 steel axe
/// remains on shelf for resumed purchase after deposit. Flat 3×200 ignores haggle.
const SHOP_BUYOUT_BOB_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_BOB_BUDGET_GP: f64 = 1500.0;
/// Nurmof nonstackable iron pickaxe: pickaxeshop sell 1000 delta 20, baseline 5 cost 140.
/// perTrip 500 → stock-sensitive 140+142+145=427 spent → 73 coins (<100 bank).
/// Stock 5 at start; no restock wait on first pass. Long mine↔Falador East route.
const SHOP_BUYOUT_NURMOF_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_NURMOF_BUDGET_GP: f64 = 1500.0;
/// Magic Guild stackable blood rune: magicguildshop baseline 1000 cost 50 members.
/// Stackable coins<100 bank trigger like Aubury; perTrip 500 / budget 1500.
const SHOP_BUYOUT_MAGIC_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_MAGIC_BUDGET_GP: f64 = 1500.0;
/// Lundail stackable fire rune: magearena_runeshop sell 1000 delta 30, baseline
/// 200 cost 4. Stock-sensitive unit_price at shelf 200: first units 4gp, rising
/// with sold count. perTrip 500 → 66 fire runes / 490gp → 10 coins (<100 bank);
/// 134 remain for resumed purchase. Cosmic baseline 20 stocks out with 124
/// leftover (restock wait) — not selected. Cellar shop↔Gundai is Chebyshev 5.
const SHOP_BUYOUT_LUNDAIL_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_LUNDAIL_BUDGET_GP: f64 = 1500.0;
/// Fernahei stackable feather: shilofishingshop sell 1000 delta 20, baseline
/// 800 cost 2. perTrip 500 → 125 feathers / 500gp → 0 coins (<100 bank);
/// 675 remain for resume. Rods baseline 5 stock out with 475 leftover — not
/// selected. Hut↔teller is Chebyshev 18.
const SHOP_BUYOUT_FERNAHEI_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_FERNAHEI_BUDGET_GP: f64 = 1500.0;
/// Betty's frozen Falador West preset requires four travel legs before a
/// resumed purchase: shop→bank→shop for the initial withdrawal, then
/// shop→bank→shop for deposit and return. The 150-dirty first-purchase
/// watch expired while the player was still progressing back with 500 coins.
///
/// These are bounded trial estimates, not measured completion times.
/// Dirty-snapshot increments are distinct from engine ticks and wall time.
/// The failed trace had 102 engine ticks through initial withdrawal and
/// 28 on the return. Its remaining 108-tile Chebyshev distance requires at
/// least 54 ticks at two tiles/tick; later energy depletion and detours
/// can increase that. It does not establish a 136-tick return lower bound.
///
/// First purchase gets 320 dirties; later deposit/return get 280 each to
/// allow for depleted energy. At the observed roughly 1.67 dirties/second,
/// a 320-dirty first purchase is about 190s. Assuming later legs complete
/// near 120 dirties each, plus setup/actions, gives roughly 365s; 420s is
/// the wall trial bound. Those future durations remain unmeasured.
/// Empty/close/further-purchase watches, other presets, runtime timeouts,
/// and global guards retain their existing limits. Falador West is preserved.
const SHOP_BUYOUT_BETTY_FIRST_PURCHASE_WATCH_TICKS: u32 = 320;
const SHOP_BUYOUT_BETTY_DEPOSIT_WATCH_TICKS: u32 = 280;
const SHOP_BUYOUT_BETTY_RETURN_WATCH_TICKS: u32 = 280;
const SHOP_BUYOUT_BETTY_DEADLINE: Duration = Duration::from_secs(420);

/// Gerrant's frozen Draynor preset needs the same four travel legs as other
/// shop buyouts (initial shop→bank→shop withdrawal, post-buy shop→bank deposit,
/// bank→shop return, resumed buy).
///
/// **Units:** per-arm `budget_ticks` are runner dirty-snapshot increments
/// (`SCRIPT_GOLD_WATCH_TICKS` docs); whole-cycle `deadline` is wall seconds.
///
/// **Route (geometry, not measured completion):** Chebyshev Gerrant stand
/// 3013,3224 ↔ Draynor approach 3092,3243 is 79 tiles → 40 engine ticks at
/// two tiles/tick on a straight leg. Nav traces use north-loop detours (7–8
/// hops per leg) and one-tile/tick once run energy drops.
///
/// **Four legs vs arms:**
/// - First-purchase watch: shop→bank (leg 1), withdraw/open, bank→shop (leg 2),
///   Trade + buy until pack holds feather — two travel legs plus banking/shop UI.
/// - Deposit watch: shop→bank (leg 3) until product in bank — one travel leg.
/// - Return watch: bank→shop keeper radius (leg 4) — one travel leg, often
///   depleted energy (same geometry as leg 2/3).
/// - Empty/close/further-purchase arms stay ordinary 150-dirty gold watches.
///
/// **Measured live7zpi0g_0 (180s wall, host 0a80e7535):** first purchase
/// 160 feathers / 488gp, deposit, empty, close, 488gp withdraw under 150-dirty
/// deposit arm; whole scenario deadline 180s exceeded on step 21 return at
/// 3075,3264 run energy 6 (315 runner increments; capture
/// `2026-09-18T11-53-30_shop_buyout_gerrant`). Establishes mid-return fail and
/// deposit pass, not a first-purchase upper bound under hitch variance.
///
/// **Measured livexjav5j_0 (105.046s, host 0c14892f5):** step 17 first-purchase
/// watch exhausted 150 dirty at 168 runner increments with 500 coins, no
/// feather, run energy 20; nav arrived 3015,3222 engine tick 165 then Trade
/// Gerrant before timeout (capture `2026-09-18T12-19-09_shop_buyout_gerrant`).
/// Proves 150 insufficient for the two-leg + withdraw + buy arm under scene
/// reload/hitch variance — not a completed first-purchase duration.
///
/// **First-purchase dirty 240:** observed arm exhausted 150 dirty while the
/// whole runner reported 168 increments (including setup). The additional
/// 90 arm increments are a bounded trial margin for remaining Trade/buy work
/// and route variation; they are not a measured UI or hitch duration.
///
/// **Deposit dirty 150:** live7zpi0g measured pass for leg 3; one depleted
/// leg is shorter than the first-purchase composite — no widening without a
/// deposit-arm fail.
///
/// **Return dirty 240:** bounded trial above depleted leg-4 lower bound plus
/// detour margin from live7zpi0g mid-return geometry (~62 Chebyshev tiles
/// remaining at 180s wall); not Betty's 280.
///
/// **Whole-cycle wall 390s:** live7zpi0g 180s through mid-return plus ~210s
/// trial remainder for depleted return and resumed buy; retained unless LIVE
/// shows deadline fail with widened first-purchase dirty budget.
const SHOP_BUYOUT_GERRANT_FIRST_PURCHASE_WATCH_TICKS: u32 = 240;
const SHOP_BUYOUT_GERRANT_RETURN_WATCH_TICKS: u32 = 240;
const SHOP_BUYOUT_GERRANT_DEADLINE: Duration = Duration::from_secs(390);

/// Per-case observation budgets for [`shop_buyout_variant`]. Default matches
/// ordinary gold watches; Betty widens only the long bank-travel arms.
struct ShopBuyoutTiming {
    first_purchase_ticks: u32,
    deposit_ticks: u32,
    return_ticks: u32,
    deadline: Duration,
}

const SHOP_BUYOUT_DEFAULT_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SCRIPT_GOLD_WATCH_TICKS,
    deposit_ticks: SCRIPT_GOLD_WATCH_TICKS,
    return_ticks: SCRIPT_GOLD_WATCH_TICKS,
    deadline: SCRIPT_GOLD_DEADLINE,
};

const SHOP_BUYOUT_BETTY_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SHOP_BUYOUT_BETTY_FIRST_PURCHASE_WATCH_TICKS,
    deposit_ticks: SHOP_BUYOUT_BETTY_DEPOSIT_WATCH_TICKS,
    return_ticks: SHOP_BUYOUT_BETTY_RETURN_WATCH_TICKS,
    deadline: SHOP_BUYOUT_BETTY_DEADLINE,
};

const SHOP_BUYOUT_GERRANT_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SHOP_BUYOUT_GERRANT_FIRST_PURCHASE_WATCH_TICKS,
    deposit_ticks: SCRIPT_GOLD_WATCH_TICKS,
    return_ticks: SHOP_BUYOUT_GERRANT_RETURN_WATCH_TICKS,
    deadline: SHOP_BUYOUT_GERRANT_DEADLINE,
};

/// Bob Lumbridge ↔ Draynor: Chebyshev stand 3231,3203 ↔ Draynor approach
/// 3092,3243 is 139 tiles (estimate, not measured) — wider than Gerrant's 79.
/// Reuses Betty's four-leg trial dirty budgets and 420s wall until LIVE
/// measures Bob-specific variance.
const SHOP_BUYOUT_BOB_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_BETTY_TIMING;

/// Nurmof livewfnjs8_0 reached the return Trade but hit the 420s deadline
/// before a fresh purchase. Summed host windows put those Trade sends at
/// 406.586/410.124/413.621s and termination at 415.941s (approximate process
/// timing, not an atomic shop-open witness). Native open waits 3s per attempt.
/// Allow one bounded 30s observation margin for the remaining open/buy work;
/// this is a trial allowance, not proof that timing is the only defect.
/// Keep every dirty budget and the fresh post-bank purchase proof unchanged.
const SHOP_BUYOUT_NURMOF_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    deadline: Duration::from_secs(450),
    ..SHOP_BUYOUT_BETTY_TIMING
};

/// Lundail cellar shop 2535,4719 ↔ Gundai stand 2533,4714 is Chebyshev 5
/// (estimate, not measured). Four legs stay inside the Mage Arena cellar —
/// not the wilderness lever/web approach. Ordinary gold 150/180s, same
/// family as Aemad/Aubury short booth routes.
const SHOP_BUYOUT_LUNDAIL_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_DEFAULT_TIMING;

/// Fernahei hut 2870,2971 ↔ Shilo teller 2852,2954 is Chebyshev 18
/// (estimate, not measured), shorter than Aemad's ~42-tile East Ardougne
/// booth route. Ordinary gold 150/180s. Village wooden gates are not on
/// this interior shop↔bank geometry.
const SHOP_BUYOUT_FERNAHEI_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_DEFAULT_TIMING;

const SHOP_BUYOUT_AEMAD_LABEL: &str =
    "Aemad's vials — East Ardougne (Ardougne East bank)";
const SHOP_BUYOUT_AUBURY_LABEL: &str = "Aubury's runes — Varrock (Varrock East bank)";
const SHOP_BUYOUT_LOWE_LABEL: &str = "Lowe's arrows — Varrock (Varrock East bank)";
const SHOP_BUYOUT_HICKTON_LABEL: &str = "Hickton's arrows — Catherby (Catherby bank)";
const SHOP_BUYOUT_HARRY_LABEL: &str = "Harry's fishing — Catherby (Catherby bank)";
const SHOP_BUYOUT_BETTY_LABEL: &str = "Betty's runes — Port Sarim (Falador West bank)";
const SHOP_BUYOUT_GERRANT_LABEL: &str = "Gerrant's feathers — Port Sarim (Draynor bank)";
const SHOP_BUYOUT_BOB_LABEL: &str = "Bob's axes — Lumbridge (Draynor bank)";
const SHOP_BUYOUT_NURMOF_LABEL: &str = "Nurmof's pickaxes — Dwarven Mine (Falador East bank)";
const SHOP_BUYOUT_MAGIC_LABEL: &str = "Wizard Guild runes — Yanille (Yanille bank)";
const SHOP_BUYOUT_LUNDAIL_LABEL: &str = "Mage Arena runes — Lundail (Gundai bank)";
const SHOP_BUYOUT_FERNAHEI_LABEL: &str = "Fernahei's fishing — Shilo Village (Shilo bank)";
const SHOP_BUYOUT_AEMAD_ITEM: &str = "Vial of water";
const SHOP_BUYOUT_AUBURY_ITEM: &str = "Fire rune";
const SHOP_BUYOUT_LOWE_ITEM: &str = "Bronze arrow";
const SHOP_BUYOUT_HICKTON_ITEM: &str = "Bronze arrow";
const SHOP_BUYOUT_HARRY_ITEM: &str = "Fishing bait";
const SHOP_BUYOUT_BETTY_ITEM: &str = "Fire rune";
const SHOP_BUYOUT_GERRANT_ITEM: &str = "Feather";
const SHOP_BUYOUT_BOB_ITEM: &str = "Steel axe";
const SHOP_BUYOUT_NURMOF_ITEM: &str = "Iron pickaxe";
const SHOP_BUYOUT_MAGIC_ITEM: &str = "Blood rune";
const SHOP_BUYOUT_LUNDAIL_ITEM: &str = "Fire rune";
const SHOP_BUYOUT_FERNAHEI_ITEM: &str = "Feather";
const FISHING_BAIT_ID: i32 = 313;
const STEEL_AXE_ID: i32 = 1353;
const IRON_PICKAXE_ID: i32 = 1267;

const AIO_TELEPORT_INJECT: &[ScriptSettingInject] = &[];
const AIO_TELEPORT_FALADOR_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "teleportName",
    value: ScriptInjectValue::Str("falador"),
}];
const AIO_TELEPORT_NO_STAFF_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "useStaffRunes",
    value: ScriptInjectValue::Bool(false),
}];
const SHOP_BUYOUT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_AEMAD_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AEMAD_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AEMAD_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_AEMAD_ITEM]),
    },
];
const SHOP_BUYOUT_AUBURY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_AUBURY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AUBURY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AUBURY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_AUBURY_ITEM]),
    },
];
const SHOP_BUYOUT_LOWE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_LOWE_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LOWE_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LOWE_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_LOWE_ITEM]),
    },
];
const SHOP_BUYOUT_HICKTON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_HICKTON_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HICKTON_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HICKTON_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_HICKTON_ITEM]),
    },
];
const SHOP_BUYOUT_HARRY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_HARRY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HARRY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HARRY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_HARRY_ITEM]),
    },
];
const SHOP_BUYOUT_BETTY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_BETTY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BETTY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BETTY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_BETTY_ITEM]),
    },
];
const SHOP_BUYOUT_GERRANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_GERRANT_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_GERRANT_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_GERRANT_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_GERRANT_ITEM]),
    },
];
const SHOP_BUYOUT_BOB_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_BOB_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BOB_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BOB_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_BOB_ITEM]),
    },
];
const SHOP_BUYOUT_NURMOF_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_NURMOF_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_NURMOF_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_NURMOF_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_NURMOF_ITEM]),
    },
];
const SHOP_BUYOUT_MAGIC_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_MAGIC_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_MAGIC_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_MAGIC_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_MAGIC_ITEM]),
    },
];
const SHOP_BUYOUT_LUNDAIL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_LUNDAIL_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LUNDAIL_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LUNDAIL_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_LUNDAIL_ITEM]),
    },
];
const SHOP_BUYOUT_FERNAHEI_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_FERNAHEI_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_FERNAHEI_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_FERNAHEI_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_FERNAHEI_ITEM]),
    },
];
const SMITHING_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
const SMITHING_BOT_PLATEBODY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Platebody"),
    },
];
const SMITHING_BOT_NAILS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Steel"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Nails"),
    },
];
const SMITHING_BOT_MITHRIL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Mithril"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
const LEATHER_CRAFTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Leather"),
}];
const LEATHER_CRAFTER_HARD_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Hard leather"),
}];
const LEATHER_CRAFTER_GREEN_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Green dragon leather"),
}];
const FIREMAKER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "logType",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Varrock East"),
    },
];
const FIREMAKER_OAK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "logType",
        value: ScriptInjectValue::Str("Oak logs"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Varrock East"),
    },
];

struct AioTeleportPlan {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    magic: i32,
    staff_id: Option<i32>,
    staff_alias: Option<&'static str>,
    pack_air: bool,
    pack_fire: bool,
    landing: WorldTile,
    restock: WorldTile,
}

fn aio_teleport_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport",
        inject: AIO_TELEPORT_INJECT,
        magic: 25,
        staff_id: Some(STAFF_OF_AIR_ID),
        staff_alias: Some("staff_of_air"),
        pack_air: false,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

fn aio_teleport_falador_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_falador",
        inject: AIO_TELEPORT_FALADOR_INJECT,
        magic: 37,
        staff_id: Some(STAFF_OF_WATER_ID),
        staff_alias: Some("staff_of_water"),
        pack_air: true,
        pack_fire: false,
        landing: FALADOR_TELE_LAND,
        restock: FALADOR_WEST_BANK,
    })
}

fn aio_teleport_no_staff_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_no_staff",
        inject: AIO_TELEPORT_NO_STAFF_INJECT,
        magic: 25,
        staff_id: None,
        staff_alias: None,
        pack_air: true,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

/// Pack two laws so the default 1000-law withdraw never runs inside 180s.
/// Falador also packs Air: water staff covers Water, not Air.
fn aio_teleport_variant(plan: AioTeleportPlan) -> Scenario {
    let AioTeleportPlan {
        name,
        inject,
        magic,
        staff_id,
        staff_alias,
        pack_air,
        pack_fire,
        landing,
        restock,
    } = plan;
    let first_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let land = Proof::ArrivedNear {
        x: landing.x,
        z: landing.z,
        level: landing.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic, packed laws, and Lumbridge bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat magic {magic}"));
                cheat(c, "give lawrune 2");
                if pack_air {
                    cheat(c, "give airrune 20");
                }
                if pack_fire {
                    cheat(c, "give firerune 20");
                }
                if let Some(alias) = staff_alias {
                    cheat(c, &format!("give {alias} 1"));
                }
                cheat(c, "givebank lawrune 200");
                cheat(
                    c,
                    &tele_args(LUMBRIDGE_BANK.level, LUMBRIDGE_BANK.x, LUMBRIDGE_BANK.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: LUMBRIDGE_BANK.x,
                z: LUMBRIDGE_BANK.z,
                level: LUMBRIDGE_BANK.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm Magic before Start",
        Proof::Stat {
            id: MAGIC_STAT,
            min: magic,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm packed laws before Start",
        Proof::ItemId {
            id: LAW_RUNE_ID,
            count: 2,
        },
    ));
    if let Some(id) = staff_id {
        steps.push(wear_combat_item_step(
            "wield and acknowledge the covering staff before Start",
            id,
        ));
    } else {
        steps.push(bank_fletcher_watch(
            "confirm no covering air staff before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_AIR_ID,
                count: 0,
            },
        ));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the banked law restock",
        Proof::BankItemId {
            id: LAW_RUNE_ID,
            count: 200,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Magic XP from Game.teleport after Start", first_xp),
        (
            "watch arrival at the selected teleport land after Start",
            land,
        ),
        (
            "watch a packed law consumed after Start",
            Proof::ItemIdAtMost {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        (
            "watch law restock at the destination bank after Start",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        ("watch the teleport bank close", Proof::BankClosed),
        ("watch a further Magic XP after restock", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let _ = restock;
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
            start_script: Some("AIO Teleport"),
            script_settings_inject: if inject.is_empty() {
                None
            } else {
                Some(inject)
            },
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Aemad nonstackable ShopBuyout: selected `Vial of water` (obj 227) only.
/// Coin seed via ordinary booth deposit; product unseeded. Natural bank on
/// pack-full needSpaceFor; budget>perTrip permits resumed purchase.
fn shop_buyout_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout",
        SHOP_BUYOUT_INJECT,
        AEMAD_STAND,
        AEMAD_BANK_APPROACH,
        AEMAD_BANK_BOOTH,
        AEMAD_BANK_BOOTH_ID,
        "Aemad",
        VIAL_OF_WATER_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Aubury stackable ShopBuyout: selected `Fire rune` (obj 554) only. Pack
/// fill does not bank an existing rune stack; natural bank is coins<100
/// after a real buy with remaining budget for withdraw+resume.
fn shop_buyout_aubury_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_aubury",
        SHOP_BUYOUT_AUBURY_INJECT,
        AUBURY_STAND,
        VARROCK_EAST_BANK_APPROACH,
        VARROCK_EAST_BANK_BOOTH,
        VARROCK_EAST_BANK_BOOTH_ID,
        "Aubury",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Lowe stackable ShopBuyout: selected `Bronze arrow` (obj 882) only. Same
/// Varrock East bank geometry as Aubury; coins<100 after real buy drives bank.
fn shop_buyout_lowe_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_lowe",
        SHOP_BUYOUT_LOWE_INJECT,
        LOWE_STAND,
        VARROCK_EAST_BANK_APPROACH,
        VARROCK_EAST_BANK_BOOTH,
        VARROCK_EAST_BANK_BOOTH_ID,
        "Lowe",
        BRONZE_ARROW_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Hickton stackable ShopBuyout: selected `Bronze arrow` (obj 882) only.
/// Catherby open booth 2213@2809,3442 with south approach 2809,3441.
fn shop_buyout_hickton_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_hickton",
        SHOP_BUYOUT_HICKTON_INJECT,
        HICKTON_STAND,
        CATHERBY_BANK_APPROACH,
        CATHERBY_BANK_BOOTH,
        CATHERBY_BANK_BOOTH_ID,
        "Hickton",
        BRONZE_ARROW_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Harry stackable ShopBuyout: selected `Fishing bait` (obj 313) only. Same
/// Catherby bank geometry as Hickton.
fn shop_buyout_harry_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_harry",
        SHOP_BUYOUT_HARRY_INJECT,
        HARRY_STAND,
        CATHERBY_BANK_APPROACH,
        CATHERBY_BANK_BOOTH,
        CATHERBY_BANK_BOOTH_ID,
        "Harry",
        FISHING_BAIT_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Betty stackable ShopBuyout: selected `Fire rune` (obj 554) only. Falador
/// West open booth 2213@2946,3367 with approach 2946,3368 (preset bankStand
/// 2946,3369 is Chebyshev 2). coins<100 after real buy drives bank.
/// Long Port Sarim↔Falador West travel uses [`SHOP_BUYOUT_BETTY_TIMING`].
fn shop_buyout_betty_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_betty",
        SHOP_BUYOUT_BETTY_INJECT,
        BETTY_STAND,
        FALADOR_WEST_BANK_APPROACH,
        FALADOR_WEST_BANK_BOOTH,
        FALADOR_WEST_BANK_BOOTH_ID,
        "Betty",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_BETTY_TIMING,
    )
}

/// Gerrant stackable ShopBuyout: selected `Feather` (obj 314) only. Reuses
/// Draynor open booth 2213@3091,3243 with approach 3092,3243 (frozen bankStand).
fn shop_buyout_gerrant_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_gerrant",
        SHOP_BUYOUT_GERRANT_INJECT,
        GERRANT_STAND,
        DRAYNOR_BANK_APPROACH,
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
        "Gerrant",
        FEATHER_ID,
        SHOP_BUYOUT_GERRANT_TIMING,
    )
}

/// Bob nonstackable ShopBuyout: selected `Steel axe` (obj 1353) only.
/// Long Lumbridge ↔ Draynor route uses [`SHOP_BUYOUT_BOB_TIMING`].
fn shop_buyout_bob_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_bob",
        SHOP_BUYOUT_BOB_INJECT,
        BOB_STAND,
        DRAYNOR_BANK_APPROACH,
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
        "Bob",
        STEEL_AXE_ID,
        SHOP_BUYOUT_BOB_TIMING,
    )
}

/// Nurmof nonstackable ShopBuyout: selected `Iron pickaxe` (obj 1267) only.
/// Dwarven Mine ↔ Falador East uses [`SHOP_BUYOUT_NURMOF_TIMING`].
fn shop_buyout_nurmof_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_nurmof",
        SHOP_BUYOUT_NURMOF_INJECT,
        NURMOF_STAND,
        FALADOR_EAST_BANK,
        FALADOR_EAST_BOOTH,
        2213,
        "Nurmof",
        IRON_PICKAXE_ID,
        SHOP_BUYOUT_NURMOF_TIMING,
    )
}

/// Magic Store owner stackable ShopBuyout: selected `Blood rune` (obj 565) only.
/// Guild floor-1 shop with Yanille booth seed; pre-Start magic 66 for return
/// legs through the guild door (`magic_guild.rs2`).
fn shop_buyout_magic_scenario() -> Scenario {
    let mut scenario = shop_buyout_variant(
        "shop_buyout_magic",
        SHOP_BUYOUT_MAGIC_INJECT,
        MAGIC_STORE_STAND,
        YANILLE_BANK_APPROACH,
        YANILLE_BANK_BOOTH,
        YANILLE_BANK_BOOTH_ID,
        "Magic Store owner",
        BLOOD_RUNE_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    );
    let tele_shop = scenario
        .steps
        .iter()
        .position(|step| step.name == "tele to the original shop keeper before Start")
        .expect("shop buyout seed tele to shop");
    scenario.steps.insert(
        tele_shop,
        Step {
            name: "seed magic 66 for Wizard Guild door on post-bank return legs",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setstat magic 66");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: 66,
                },
                budget_ticks: 200,
            },
        },
    );
    scenario.steps.insert(tele_shop + 1, drain_advancestat());
    scenario
}

/// Lundail stackable ShopBuyout: selected `Fire rune` (obj 554) only.
/// Coin seed through Gundai Talk-to + exact cellar bank choice. Shop↔bank
/// stays in the Mage Arena cellar (Chebyshev 5); wilderness lever/webs are
/// not on the proof cycle.
fn shop_buyout_lundail_scenario() -> Scenario {
    shop_buyout_with_bank(
        "shop_buyout_lundail",
        SHOP_BUYOUT_LUNDAIL_INJECT,
        LUNDAIL_STAND,
        ShopBuyoutBankAccess::Npc {
            approach: GUNDAI_BANK_APPROACH,
            banker: "Gundai",
            action: "Talk-to",
            choose: "Cool, I'd like to access my bank account please.",
        },
        "Lundail",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_LUNDAIL_TIMING,
    )
}

/// Fernahei stackable ShopBuyout: selected `Feather` (obj 314) only.
/// Coin seed through the Shilo `Banker` Talk-to + exact teller choice.
/// Village membership is the native Shilo Village journal acknowledgement
/// before the coin seed; shop↔bank is interior Chebyshev 18.
fn shop_buyout_fernahei_scenario() -> Scenario {
    let mut scenario = shop_buyout_with_bank(
        "shop_buyout_fernahei",
        SHOP_BUYOUT_FERNAHEI_INJECT,
        FERNAHEI_STAND,
        ShopBuyoutBankAccess::Npc {
            approach: SHILO_BANK_APPROACH,
            banker: "Banker",
            action: "Talk-to",
            choose: "I'd like to access my bank account, please.",
        },
        "Fernahei",
        FEATHER_ID,
        SHOP_BUYOUT_FERNAHEI_TIMING,
    );
    let coin = scenario
        .steps
        .iter()
        .position(|step| {
            step.name
                == "seed stackable coins and stand at the operable shop bank approach before Start"
        })
        .expect("shop buyout coin seed");
    for step in quest_prereq_steps(SHILO_VILLAGE_PREREQ).into_iter().rev() {
        scenario.steps.insert(coin, step);
    }
    scenario
}

/// Seed-bank identity for [`shop_buyout_with_bank`]. Booth fixtures keep the
/// exact Use-quickly loc path; NPC fixtures Talk-to a named teller and
/// press that teller's authentic bank choice. Deposit/close stay shared.
enum ShopBuyoutBankAccess {
    Booth {
        approach: WorldTile,
        booth: WorldTile,
        booth_id: i32,
    },
    Npc {
        approach: WorldTile,
        banker: &'static str,
        action: &'static str,
        choose: &'static str,
    },
}

fn shop_buyout_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    stand: WorldTile,
    bank_approach: WorldTile,
    booth: WorldTile,
    booth_id: i32,
    keeper_name: &'static str,
    product_id: i32,
    timing: ShopBuyoutTiming,
) -> Scenario {
    shop_buyout_with_bank(
        name,
        inject,
        stand,
        ShopBuyoutBankAccess::Booth {
            approach: bank_approach,
            booth,
            booth_id,
        },
        keeper_name,
        product_id,
        timing,
    )
}

#[derive(Default)]
struct ShopBuyoutNpcOpenState {
    talked: bool,
    saw_chat: bool,
    continued: Option<(i32, i32, String)>,
    answered: Option<(i32, String)>,
}

fn shop_buyout_open_npc_bank(
    name: &'static str,
    arm: Proof,
    banker: &'static str,
    approach: WorldTile,
    action: &'static str,
    choose: &'static str,
) -> Step {
    let state = std::sync::Mutex::new(ShopBuyoutNpcOpenState::default());
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                let Ok(mut state) = state.lock() else {
                    return false;
                };
                if snapshot.bank_component_id() >= 0 {
                    return true;
                }
                let chat_open = snapshot.modals().chat != -1;
                if chat_open {
                    state.saw_chat = true;
                }
                if snapshot.chat_continue_component_id() != -1 {
                    let identity = (
                        snapshot.modals().chat,
                        snapshot.chat_continue_component_id(),
                        snapshot.chat_modal_texts().join("\n"),
                    );
                    if state.continued.as_ref() == Some(&identity) {
                        return true;
                    }
                    let mut ix = Interactions::new(snapshot, c);
                    return match ix.continue_dialog() {
                        SendResult::Sent { .. } => {
                            state.continued = Some(identity);
                            true
                        }
                        SendResult::Refused { .. } => true,
                    };
                }
                if !snapshot.chat_options().is_empty() {
                    let identity = (
                        snapshot.modals().chat,
                        snapshot
                            .chat_options()
                            .iter()
                            .map(|option| option.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    if state.answered.as_ref() == Some(&identity) {
                        return true;
                    }
                    let Some(choice) = snapshot
                        .chat_options()
                        .iter()
                        .position(|option| option.text == choose)
                    else {
                        return false;
                    };
                    let mut ix = Interactions::new(snapshot, c);
                    return match ix.answer_choice(choice as i32 + 1) {
                        SendResult::Sent { .. } => {
                            state.answered = Some(identity);
                            true
                        }
                        SendResult::Refused { .. } => true,
                    };
                }
                if chat_open || (state.talked && !state.saw_chat) || state.answered.is_some() {
                    return true;
                }
                if state.talked && state.saw_chat {
                    *state = ShopBuyoutNpcOpenState::default();
                }
                let Some(npc) = snapshot.npcs().iter().find(|npc| {
                    npc.name.as_deref() == Some(banker)
                        && npc.tile.level == approach.level
                        && (npc.tile.x - approach.x)
                            .abs()
                            .max((npc.tile.z - approach.z).abs())
                            <= 12
                        && npc.actions.iter().any(|published| {
                            published
                                .as_deref()
                                .is_some_and(|label| label.eq_ignore_ascii_case(action))
                        })
                }) else {
                    return true;
                };
                let Some((px, pz, level)) = snapshot.tile() else {
                    return true;
                };
                if level != npc.tile.level
                    || (px - npc.tile.x).abs().max((pz - npc.tile.z).abs()) > 1
                {
                    let mut ix = Interactions::new(snapshot, c);
                    return matches!(
                        ix.walk(npc.tile),
                        SendResult::Sent { .. } | SendResult::Refused { .. }
                    );
                }
                let mut ix = Interactions::new(snapshot, c);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label(action.to_string())) {
                    SendResult::Sent { .. } => {
                        state.talked = true;
                        true
                    }
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { reason, .. } => {
                        eprintln!(
                            "[scenario] npc bank {banker} {action} send refused: {reason:?}"
                        );
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

fn shop_buyout_with_bank(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    stand: WorldTile,
    bank: ShopBuyoutBankAccess,
    keeper_name: &'static str,
    product_id: i32,
    timing: ShopBuyoutTiming,
) -> Scenario {
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let coin_seed = SHOP_BUYOUT_COIN_SEED;
    let bank_approach = match bank {
        ShopBuyoutBankAccess::Booth { approach, .. }
        | ShopBuyoutBankAccess::Npc { approach, .. } => approach,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed stackable coins and stand at the operable shop bank approach before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give coins {coin_seed}"));
                cheat(
                    c,
                    &tele_args(bank_approach.level, bank_approach.x, bank_approach.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank_approach.x,
                z: bank_approach.z,
                level: bank_approach.level,
                radius: 4,
            },
            budget_ticks: 200,
        },
    });
    match bank {
        ShopBuyoutBankAccess::Booth {
            booth,
            booth_id,
            ..
        } => {
            steps.push(bank_fletcher_watch(
                "acknowledge exact shop bank booth identity and Use-quickly action before bank send",
                Proof::LocActionNear {
                    id: booth_id,
                    x: booth.x,
                    z: booth.z,
                    level: booth.level,
                    radius: 0,
                    action: "Use-quickly",
                    present: true,
                },
            ));
            // Same exact-booth open pattern as Herblore readiness (do not retarget
            // closed booths via open_nearest). Reuses the generic helper only.
            steps.push(herblore_open_seed_bank_at(
                "open the exact named shop bank booth for the coin seed deposit",
                Proof::BankItemIdAtMost {
                    id: COINS_ID,
                    count: 0,
                },
                booth,
                booth_id,
            ));
        }
        ShopBuyoutBankAccess::Npc {
            banker,
            action,
            choose,
            ..
        } => {
            steps.push(bank_fletcher_watch(
                "acknowledge the exact named shop banker and Talk-to action before bank send",
                Proof::NpcNameNear {
                    name: banker,
                    x: bank_approach.x,
                    z: bank_approach.z,
                    level: bank_approach.level,
                    radius: 12,
                },
            ));
            steps.push(shop_buyout_open_npc_bank(
                "open the exact named shop banker for the coin seed deposit",
                Proof::BankItemIdAtMost {
                    id: COINS_ID,
                    count: 0,
                },
                banker,
                bank_approach,
                action,
                choose,
            ));
        }
    }
    steps.extend(native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![NativeSeed {
            unnoted_id: COINS_ID,
            debug_alias: "coins",
            note_alias: None,
            quantity: coin_seed,
            note_id: None,
        }],
    ));
    for (step_name, arm) in [
        (
            "confirm the named bank holds the exact coin seed",
            Proof::BankItemId {
                id: COINS_ID,
                count: coin_seed,
            },
        ),
        (
            "confirm the named bank has no excess coin seed",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: coin_seed,
            },
        ),
        (
            "confirm no seeded product in bank before Start",
            Proof::BankItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm the coin seed left the pack",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "tele to the original shop keeper before Start",
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
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge the original shop keeper before Start",
        Proof::NpcNameNear {
            name: keeper_name,
            x: stand.x,
            z: stand.z,
            level: stand.level,
            radius: 12,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm pack still empty of product at the shop before Start",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    // Contract: unseeded product → fresh bank product → empty pack →
    // BankClosed → return named shop → new product. Coins secondary only.
    // Travel-leg dirty budgets come from `timing` (Betty long West route);
    // empty/close/further stay ordinary gold watches. See
    // SHOP_BUYOUT_BETTY_* constant docs (dirty increments, not 150×600ms).
    for (step_name, arm) in [
        (
            "watch unseeded purchased product in pack after Start",
            product,
        ),
        (
            "watch purchased product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch the buyout bank close after deposit",
            Proof::BankClosed,
        ),
        (
            "watch return to the named shop after banking",
            Proof::NpcNameNear {
                name: keeper_name,
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 12,
            },
        ),
        (
            "watch further purchased product after return",
            product,
        ),
    ] {
        let budget_ticks = match step_name {
            "watch unseeded purchased product in pack after Start" => {
                timing.first_purchase_ticks
            }
            "watch purchased product enter a fresh bank" => timing.deposit_ticks,
            "watch return to the named shop after banking" => timing.return_ticks,
            _ => SCRIPT_GOLD_WATCH_TICKS,
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm,
                budget_ticks,
            },
        });
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
            deadline: timing.deadline,
            start_script: Some("ShopBuyout"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// ClimbingBoots option cells. `useTeleport` and `runeStock` are typed on
/// purpose: the frozen script's defaults are true/50, and the walking cell
/// must be the explicit false branch, the teleport cell the explicit true
/// branch with the smallest non-zero rune stock.
const CLIMBING_BOOTS_WALK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];
const CLIMBING_BOOTS_TELEPORT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];

const CLIMBING_BOOTS_ID: i32 = 3105;
const CLIMBING_BOOTS_PAIR_COINS: i32 = 12;
const CLIMBING_BOOTS_TELE_MAGIC: i32 = 37;
/// `readyToBuy` wants coins === 12 * tripQty with no unrelated inventory.
/// tripQty is 28 minus the carried rune stacks, so the walk cell carries
/// 28 pairs and the teleport cell 25 pairs beside Law 1/Air 3/Water 1.
const CLIMBING_BOOTS_WALK_PACK_COINS: i32 = 28 * CLIMBING_BOOTS_PAIR_COINS;
const CLIMBING_BOOTS_TELE_PACK_COINS: i32 = 25 * CLIMBING_BOOTS_PAIR_COINS;
/// Bank stock for the later withdrawals the full cycle claims: two further
/// trips of coins, and — teleport cell only — the exact Law 1/Air 3/Water 1
/// restock, because `runeStock=1` consumes the whole carried stack per cast.
const CLIMBING_BOOTS_BANK_TRIPS: i32 = 2;
const CLIMBING_BOOTS_RUNES: &[(&str, i32)] = &[("lawrune", 1), ("airrune", 3), ("waterrune", 1)];
/// The Water rune id for the bank-seed acknowledgement (Law 563 and Air 556
/// already have crate constants).
const WATER_RUNE_ID: i32 = 555;
/// Tenzing's hut: the door tile the pack stands on and the inside tile the
/// frozen script targets. Route/prep targets only, never PASS predicates.
const TENZING_DOOR: WorldTile = WorldTile {
    x: 2823,
    z: 3555,
    level: 0,
};
const TENZING_INSIDE: WorldTile = WorldTile {
    x: 2820,
    z: 3556,
    level: 0,
};
const TENZING_NAME: &str = "Tenzing";

/// The bank-side op slot whose label is the bulk deposit (`Deposit All`),
/// 1-based, matched the way the host's own deposit path does (case and
/// separator insensitive). This is the ordinary bank window the player uses.
fn bank_deposit_all_op(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|action| {
            action.as_deref().is_some_and(|label| {
                let normalised: String = label
                    .to_ascii_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
                    .collect();
                normalised.contains("deposit") && normalised.contains("all")
            })
        })
        .map(|slot| slot as i32 + 1)
}

fn climbing_boots_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots",
        CLIMBING_BOOTS_WALK_INJECT,
        CLIMBING_BOOTS_WALK_PACK_COINS,
        false,
    )
}

fn climbing_boots_teleport_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots_teleport",
        CLIMBING_BOOTS_TELEPORT_INJECT,
        CLIMBING_BOOTS_TELE_PACK_COINS,
        true,
    )
}

/// Clean pack at Tenzing's hut: Death Plateau completed through the authentic
/// primary `setvar death_equiproom 80` **and** retained map progress
/// `setvar death_map 8`, each with its own native `getvar` Chat receipt, a
/// relog so the journal repaints, then the exact carried trip money (plus the
/// rune stack for the teleport cell) and a bank stock for later withdrawals.
///
/// Authentic completion keeps map progress: the commander path reaches
/// `denulth_has_map` only when primary is 70 and `death_get_map >= 8`
/// (`death_scouted_area`); the completion queue then sets primary 80 and
/// leaves `death_map` unchanged. Tenzing door gates read bits 0..3 of that
/// same varp. Seeding primary alone leaves map 0 and is not a completed state.
///
/// Both `getvar` readbacks (`get death_equiproom: 80` and `get death_map: 8`),
/// the `Death Plateau` journal row and the framed `Tenzing` NPC are the
/// fixture's fail-closed prerequisite: they are exact server Chat replies to
/// the cheat path, not a client varp snapshot (default published 315 stays 0
/// and does not prove transmission). A pack that does not provide them times
/// this cell out before Start instead of seeding a shortcut. There is no
/// revision switch in this runner and none is invented here; the guard is the
/// authenticated content those steps resolve.
///
/// The pack itself carries zero boots. The bank stock is a real booth session
/// (the ordinary window, the inventory's bulk deposit op, a real close) — not
/// `givebank` and not a bank-side cheat — and it is not a purchase claim.
/// The open seed session also acknowledges `BankItemIdAtMost` boots 3105 = 0
/// before close, so leftover banked boots cannot hide behind a closed-bank
/// Start snapshot. Start is the real frozen script; nothing intervenes after
/// it, and the purchase, return, deposit and further stages are watched
/// separately. No boots, extra cash, reward XP, or unrelated progress is
/// granted in preparation.
fn climbing_boots_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    pack_coins: i32,
    use_teleport: bool,
) -> Scenario {
    let door = Proof::ArrivedNear {
        x: TENZING_DOOR.x,
        z: TENZING_DOOR.z,
        level: TENZING_DOOR.level,
        radius: 4,
    };
    let tenzing = Proof::NpcNameNear {
        name: TENZING_NAME,
        x: TENZING_INSIDE.x,
        z: TENZING_INSIDE.z,
        level: TENZING_INSIDE.level,
        radius: 12,
    };
    let bank = FALADOR_WEST_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps: Vec<Step> = Vec::new();
    // Authentic completed Death Plateau: primary 80 + retained map progress 8
    // (bits 0..3). Separate setvar/getvar + Chat receipts so each value is
    // proven by the server reply before relog/Start — not a client snapshot.
    steps.push(Step {
        name: "complete Death Plateau primary by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_equiproom 80");
                cheat(c, "getvar death_equiproom");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_equiproom: 80",
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "retain Death Plateau map progress by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_map 8");
                cheat(c, "getvar death_map");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_map: 8",
            },
            budget_ticks: 200,
        },
    });
    steps.extend(script_live_seed_steps());
    steps.push(bank_fletcher_watch(
        "acknowledge Death Plateau complete before Start",
        Proof::QuestDone {
            name: "Death Plateau",
        },
    ));
    let booth = FALADOR_WEST_BOOTH;
    let bank_coins = CLIMBING_BOOTS_BANK_TRIPS * pack_coins;
    steps.push(Step {
        name: "seed the bank-bound stack and stand at the Falador West booth",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give coins {bank_coins}"));
                if use_teleport {
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &tele_args(booth.level, booth.x, booth.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: booth.x,
                z: booth.z,
                level: booth.level,
                radius: 4,
            },
            budget_ticks: 200,
        },
    });
    // The bank seed is a real session, not a bank-side cheat: the ordinary
    // booth window, the inventory's bulk deposit op, and a real close. The
    // deposited rows are the same stackables the script later withdraws.
    steps.push(open_seed_booth(
        "open the real Falador West bank for the seed deposit",
        booth,
        Proof::BankItemId {
            id: COINS_ID,
            count: 0,
        },
    ));
    steps.push(Step {
        name: "deposit the seeded coins and runes through the bank window",
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                // Repeat sends before checking its arm. Once the deposit has
                // landed the bank-side pack is empty; acknowledge the actual
                // fresh-bank readback instead of refusing an unnecessary send.
                if (Proof::BankItemId {
                    id: COINS_ID,
                    count: bank_coins,
                })
                .check(snapshot, None)
                {
                    return true;
                }
                let mut ix = Interactions::new(snapshot, c);
                let mut wrote = false;
                for item in snapshot.bank_side() {
                    if let Some(op) = bank_deposit_all_op(&item.actions) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                            SendResult::Sent { .. }
                        );
                    }
                }
                wrote
            }),
        },
        wait: Wait {
            arm: Proof::BankItemId {
                id: COINS_ID,
                count: bank_coins,
            },
            budget_ticks: 200,
        },
    });
    if use_teleport {
        // `runeStock=1` spends the whole carried stack on the first cast, so
        // the seed bank itself must be acknowledged to hold the exact restock
        // the full cycle withdraws (`CLIMBING_BOOTS_RUNES`) before the session
        // closes and Start inherits it. Coins alone would let a rune-less bank
        // seed pass.
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Law rune",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Air runes",
            Proof::BankItemId {
                id: AIR_RUNE_ID,
                count: 3,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Water rune",
            Proof::BankItemId {
                id: WATER_RUNE_ID,
                count: 1,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the seed bank holds no leftover climbing boots",
        Proof::BankItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed the exact trip stack and stand at Tenzing's hut",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                if use_teleport {
                    cheat(c, &format!("setstat magic {CLIMBING_BOOTS_TELE_MAGIC}"));
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &format!("give coins {pack_coins}"));
                cheat(
                    c,
                    &tele_args(TENZING_DOOR.level, TENZING_DOOR.x, TENZING_DOOR.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: door,
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded boots in the pack before Start",
        Proof::ItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm the real Tenzing NPC is framed at the hut before Start",
        tenzing,
    ));
    steps.push(start_catalog_step());
    let mut watches: Vec<(&'static str, Proof)> = vec![
        (
            "watch the real purchase gain a pair of boots",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
        ),
        (
            "watch the purchase spend 12 coins a pair",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: pack_coins - CLIMBING_BOOTS_PAIR_COINS * 2,
            },
        ),
        ("watch the return to the Falador West bank", returned),
        (
            "watch Falador West bank hold the deposited boots",
            Proof::BankItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
        ),
        (
            "watch the bank close after restocking for a further trip",
            Proof::BankClosed,
        ),
        (
            "watch a further pair for the full cycle",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 2,
            },
        ),
    ];
    if use_teleport {
        watches.insert(
            0,
            (
                "watch the real Falador cast land at the bank",
                Proof::ArrivedNear {
                    x: FALADOR_TELE_LAND.x,
                    z: FALADOR_TELE_LAND.z,
                    level: FALADOR_TELE_LAND.level,
                    radius: 8,
                },
            ),
        );
    }
    for (step_name, arm) in watches {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let proof = if use_teleport {
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 1,
        }
    } else {
        Proof::ItemId {
            id: CLIMBING_BOOTS_ID,
            count: 2,
        }
    };
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ClimbingBoots"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn smithing_bot_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot",
        SMITHING_BOT_INJECT,
        1,
        BRONZE_DAGGER_ID,
        BRONZE_PLATEBODY_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        28,
        1,
    )
}

fn smithing_bot_platebody_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_platebody",
        SMITHING_BOT_PLATEBODY_INJECT,
        18,
        BRONZE_PLATEBODY_ID,
        BRONZE_DAGGER_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        30,
        1,
    )
}

/// Steel Nails: steel-only special panel slot, stackable out=2/bar, F2P.
/// 28 steel bars → first trip 27 bars (+ hammer) then remaining 1 bar restock.
fn smithing_bot_nails_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_nails",
        SMITHING_BOT_NAILS_INJECT,
        STEEL_NAILS_SMITHING,
        STEEL_NAILS_ID,
        BRONZE_DAGGER_ID,
        STEEL_BAR_ID,
        STEEL_BAR_CERT_ID,
        "steel_bar",
        "cert_steel_bar",
        28,
        STEEL_NAILS_OUTPUT,
    )
}

/// Mithril Dagger: 1-bar F2P product on a higher metal tier (lvl 50 / id 1209).
fn smithing_bot_mithril_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_mithril",
        SMITHING_BOT_MITHRIL_INJECT,
        MITHRIL_SMITHING,
        MITHRIL_DAGGER_ID,
        BRONZE_DAGGER_ID,
        MITHRIL_BAR_ID,
        MITHRIL_BAR_CERT_ID,
        "mithril_bar",
        "cert_mithril_bar",
        28,
        1,
    )
}

fn smithing_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    product_id: i32,
    wrong_id: i32,
    bar_id: i32,
    bar_cert_id: i32,
    bar_alias: &'static str,
    bar_note_alias: &'static str,
    bar_quantity: i32,
    product_count: i32,
) -> Scenario {
    let anvil = Proof::ArrivedNear {
        x: VARROCK_ANVIL.x,
        z: VARROCK_ANVIL.z,
        level: VARROCK_ANVIL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: product_count,
    };
    let xp = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Smithing, banked hammer/bars, and tele to Varrock West before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
        "smithing",
        smithing,
    ));
    for (step_name, arm) in [
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm the exact noted bar seed in pack before deposit",
            Proof::ItemId {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed in pack before deposit",
            Proof::ItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the noted bar seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "bound the hammer seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the bar seed bank",
        Proof::BankItemIdAtMost {
            id: bar_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the hammer and native note seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the hammer seed bank",
            Proof::BankItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact bar seed bank",
            Proof::BankItemId {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed was removed from pack",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 0,
            },
        ),
        (
            "confirm the noted bar seed was removed from pack",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
        (
            "bound the hammer seed bank count",
            Proof::BankItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the bar seed bank count",
            Proof::BankItemIdAtMost {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm no noted bars remain in bank",
            Proof::BankItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Varrock anvil after Start", anvil),
        ("watch Smithing XP from the anvil panel after Start", xp),
        ("watch the selected smithing product after Start", product),
        (
            "watch script-smithed product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: product_count,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of bars",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        ("watch the smithing bank close", Proof::BankClosed),
        ("watch return to the anvil after restock", anvil),
        ("watch further Smithing XP after restock", further_xp),
    ] {
        // Deposit arms on the first product while ≤26 bars may still forge.
        // SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS is runner dirty increments
        // (not engine p_delay ticks); see constant docs. Other arms keep
        // the ordinary gold watch; do not loosen the global deadline.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm,
                budget_ticks,
            },
        });
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
            start_script: Some("SmithingBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn leather_crafter_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter",
        LEATHER_CRAFTER_INJECT,
        1,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        28,
        LEATHER_GLOVES_ID,
        HARDLEATHER_BODY_ID,
    )
}

fn leather_crafter_hard_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_hard_body",
        LEATHER_CRAFTER_HARD_INJECT,
        28,
        "hard_leather",
        "cert_hard_leather",
        HARD_LEATHER_ID,
        HARD_LEATHER_CERT_ID,
        28,
        HARDLEATHER_BODY_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Green body at Crafting 63: multi3 make-X / chat count-dialog branch.
/// 3 leather per body; needle+thread leave 26 slots so a trip may leave 2 leather.
fn leather_crafter_green_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_green_body",
        LEATHER_CRAFTER_GREEN_INJECT,
        63,
        "dragon_leather",
        "cert_dragon_leather",
        GREEN_DRAGON_LEATHER_ID,
        GREEN_DRAGON_LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        DRAGONHIDE_BODY_ID,
        DRAGONHIDE_CHAPS_ID,
    )
}

/// Soft leather chaps at Crafting 18: selected289 uses make-10 button 8645.
fn leather_crafter_chaps_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_chaps",
        LEATHER_CRAFTER_INJECT,
        18,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        LEATHER_CHAPS_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Missing-thread purchase + return: level-1 Leather gloves with default
/// `threadPerTrip` 100. Bank holds needle, coins, and raw soft leather only —
/// absolutely no thread or crafted product in pack or bank before Start.
/// Script `fundThread` withdraws up to `threadPerTrip * THREAD_MAX_PRICE`
/// (100×3), walks nearest Dommik (3322,3194), `Shop.buy(Thread)`, closes,
/// returns to the remembered Al-Kharid bank stand, restocks leather, crafts.
///
/// Post-Start scope is purchase / return / resumed craft — not a second full
/// product bank cycle. `BankClosed` is the existing bank-modal gate only; there
/// is no `ShopClosed` proof, so shop-close is not asserted here.
fn leather_crafter_thread_shop_scenario() -> Scenario {
    let product = Proof::ItemId {
        id: LEATHER_GLOVES_ID,
        count: 1,
    };
    let xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/coins/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
        "crafting",
        1,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm zero thread in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded gloves product in pack before Start",
            Proof::ItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm coin removal",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in pack after deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in bank before Start",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no gloves product in bank before Start",
            Proof::BankItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no wrong product in bank before Start",
            Proof::BankItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: LEATHER_CERT_ID,
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
            "watch thread acquired from the zero-thread baseline after Start",
            Proof::ItemId {
                id: THREAD_ID,
                count: 1,
            },
        ),
        (
            // fundThread withdraws at most threadPerTrip*THREAD_MAX_PRICE (300).
            // After Shop.buy the pack holds strictly less than that withdrawal
            // while leftover coins remain, matching climbing_boots / shop_buyout
            // ItemIdAtMost spend arms. Not a ShopClosed proof.
            "watch coin expenditure after thread purchase",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_FUND - 1,
            },
        ),
        (
            "watch return near the original Al-Kharid bank after acquisition",
            returned,
        ),
        (
            "watch the leather bank close after restock",
            Proof::BankClosed,
        ),
        ("watch fresh Crafting XP after purchase return", xp),
        ("watch exact leather gloves after resumed craft", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "leather_crafter_thread_shop",
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
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(LEATHER_CRAFTER_INJECT),
            terminal_shot: Some("leather_crafter_thread_shop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn leather_crafter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    crafting: i32,
    leather_alias: &'static str,
    leather_note_alias: &'static str,
    leather_id: i32,
    leather_note: i32,
    leather_qty: i32,
    product_id: i32,
    wrong_id: i32,
) -> Scenario {
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/thread/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
        "crafting",
        crafting,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: crafting,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the thread seed in pack before deposit",
            Proof::ItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: leather_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the thread seed bank",
            Proof::BankItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm thread removal",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed bank count",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Crafting XP after Start", xp),
        ("watch the leather product after Start", product),
        (
            "watch script-crafted product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of leather",
            Proof::ItemId {
                id: leather_id,
                count: 1,
            },
        ),
        ("watch the leather bank close", Proof::BankClosed),
        ("watch further Crafting XP after restock", further_xp),
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
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn firemaker_scenario() -> Scenario {
    firemaker_variant(
        "firemaker",
        FIREMAKER_INJECT,
        1,
        "logs",
        LOGS_ID,
        OAK_LOGS_ID,
    )
}

fn firemaker_oak_scenario() -> Scenario {
    firemaker_variant(
        "firemaker_oak",
        FIREMAKER_OAK_INJECT,
        15,
        "oak_logs",
        OAK_LOGS_ID,
        LOGS_ID,
    )
}

fn firemaker_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    firemaking: i32,
    log_alias: &'static str,
    log_id: i32,
    wrong_id: i32,
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: FIREMAKING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: FIREMAKING_STAT,
        min: 1,
    };
    let bank = VARROCK_EAST_BANK;
    let mut steps = script_live_seed_steps();
    let log_note = if log_alias == "logs" {
        LOGS_CERT_ID
    } else {
        OAK_LOGS_CERT_ID
    };
    steps.push(native_bank_seed(
        "seed Firemaking, banked tinderbox/logs, and tele to Varrock East before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: TINDERBOX_ID,
                debug_alias: "tinderbox",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: log_id,
                debug_alias: log_alias,
                note_alias: Some(if log_alias == "logs" {
                    "cert_logs"
                } else {
                    "cert_oak_logs"
                }),
                quantity: 28,
                note_id: Some(log_note),
            },
        ],
        "firemaking",
        firemaking,
    ));
    for (step_name, arm) in [
        (
            "confirm Firemaking before Start",
            Proof::Stat {
                id: FIREMAKING_STAT,
                min: firemaking,
            },
        ),
        (
            "confirm the exact noted log seed in pack before deposit",
            Proof::ItemId {
                id: log_note,
                count: 28,
            },
        ),
        (
            "confirm the tinderbox seed in pack before deposit",
            Proof::ItemId {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "bound the noted log seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: log_note,
                count: 28,
            },
        ),
        (
            "bound the tinderbox seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded wrong logs in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the log seed bank",
        Proof::BankItemIdAtMost {
            id: log_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the tinderbox and native note log seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: TINDERBOX_ID,
                debug_alias: "tinderbox",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: log_id,
                debug_alias: log_alias,
                note_alias: Some(if log_alias == "logs" {
                    "cert_logs"
                } else {
                    "cert_oak_logs"
                }),
                quantity: 28,
                note_id: Some(log_note),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact log seed bank",
            Proof::BankItemId {
                id: log_id,
                count: 28,
            },
        ),
        (
            "acknowledge the tinderbox seed bank",
            Proof::BankItemId {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm noted log removal",
            Proof::ItemIdAtMost {
                id: log_note,
                count: 0,
            },
        ),
        (
            "confirm tinderbox removal",
            Proof::ItemIdAtMost {
                id: TINDERBOX_ID,
                count: 0,
            },
        ),
        (
            "bound the log seed bank count",
            Proof::BankItemIdAtMost {
                id: log_id,
                count: 28,
            },
        ),
        (
            "bound the tinderbox seed bank count",
            Proof::BankItemIdAtMost {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm no noted logs remain in bank",
            Proof::BankItemIdAtMost {
                id: log_note,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Firemaking XP after Start", xp),
        (
            "watch logs consumed after Start",
            Proof::ItemIdAtMost {
                id: log_id,
                count: 27,
            },
        ),
        (
            "watch a restock of logs",
            Proof::ItemId {
                id: log_id,
                count: 1,
            },
        ),
        ("watch the fire bank close", Proof::BankClosed),
        ("watch further Firemaking XP after restock", further_xp),
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
            start_script: Some("Firemaker"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
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

#[derive(Clone, Copy)]
enum PairCompanionKind {
    AirRunner,
    MuleMule,
    FlaxSpinner,
    DuelPeer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PairPrepStage {
    WaitMainland,
    TutSkip,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    AckBank,
    CloseBank,
    FirstLoad,
    WaitReady,
    Wear,
    WaitWear,
    Idle,
}

struct PairCompanionSlot {
    kind: PairCompanionKind,
    stage: PairPrepStage,
    logout_sent: bool,
    saw_logout: bool,
    logout_session: u64,
    seed_sent: bool,
    first_load_sent: bool,
    last_action: Instant,
}

impl PairCompanionSlot {
    fn new(kind: PairCompanionKind) -> Self {
        Self {
            kind,
            stage: PairPrepStage::WaitMainland,
            logout_sent: false,
            saw_logout: false,
            logout_session: 0,
            seed_sent: false,
            first_load_sent: false,
            last_action: Instant::now() - Duration::from_secs(1),
        }
    }
}

fn pair_companion_frame(c: &mut Client, slot: &mut PairCompanionSlot) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let now = Instant::now();
    let inv_tab = snap
        .side_tabs()
        .iter()
        .any(|tab| tab.index == 3 && tab.available);
    match slot.stage {
        PairPrepStage::WaitMainland => {
            if c.ingame && c.scene_state == 2 && c.map_build_base_x >= 3000 {
                slot.stage = PairPrepStage::TutSkip;
            }
        }
        PairPrepStage::TutSkip => {
            if !send_ok(slot, now) {
                return;
            }
            cheat(c, "setvar tutorial 1000");
            cheat(c, "getvar tutorial");
            slot.last_action = now;
            slot.stage = PairPrepStage::Relog;
        }
        PairPrepStage::Relog => {
            if c.ingame && !slot.logout_sent {
                if !send_ok(slot, now) {
                    return;
                }
                let ifaces = std::sync::Arc::clone(&c.ifaces);
                if logout(c, ifaces.as_slice()) {
                    slot.logout_sent = true;
                    slot.logout_session = c.gens.session;
                    slot.last_action = now;
                    slot.stage = PairPrepStage::WaitRelog;
                }
            }
        }
        PairPrepStage::WaitRelog => {
            // Host::run_client probe returns on !ingame before the next
            // observe, so companion_tick never sees the off-world frame.
            // A successful login/reconnect bumps c.gens.session; that
            // change since intentional logout is the delivered signal.
            // last_login_reconnect may already be true from an earlier
            // grant and must not admit stale pre-logout scene2.
            if pair_relog_seen(
                slot.logout_sent,
                slot.saw_logout,
                snap.ingame(),
                slot.logout_session,
                c.gens.session,
            ) {
                slot.saw_logout = true;
                if !snap.ingame() {
                    return;
                }
            }
            if slot.saw_logout && snap.ingame() && snap.scene_state() == 2 && inv_tab {
                slot.stage = PairPrepStage::Seed;
            }
        }
        PairPrepStage::Seed => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.seed_sent {
                cheat(c, "~clearinv");
                match slot.kind {
                    PairCompanionKind::AirRunner => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::MuleMule => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::FlaxSpinner => {
                        cheat(c, "setstat crafting 10");
                        cheat(c, &tele_args(FLAX_MEET.level, FLAX_MEET.x, FLAX_MEET.z));
                    }
                    PairCompanionKind::DuelPeer => {
                        cheat(c, "give bronze_scimitar 1");
                        cheat(
                            c,
                            &tele_args(DUEL_CHALLENGE.level, DUEL_CHALLENGE.x, DUEL_CHALLENGE.z),
                        );
                    }
                }
                slot.seed_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitSeed;
        }
        PairPrepStage::WaitSeed => match slot.kind {
            PairCompanionKind::AirRunner | PairCompanionKind::MuleMule => {
                if pair_near(&snap, FALADOR_EAST_BANK, 8) {
                    slot.stage = PairPrepStage::AckBank;
                }
            }
            PairCompanionKind::FlaxSpinner => {
                if pair_near(&snap, FLAX_MEET, 8)
                    && pair_stat(&snap, CRAFTING_STAT) >= 10
                    && pair_inv_id(&snap, FLAX_ID) == 0
                    && pair_inv_id(&snap, BOW_STRING_ID) == 0
                {
                    slot.stage = PairPrepStage::Idle;
                }
            }
            PairCompanionKind::DuelPeer => {
                if pair_near(&snap, DUEL_CHALLENGE, 8) && pair_inv_any(&snap) {
                    slot.stage = PairPrepStage::Wear;
                }
            }
        },
        PairPrepStage::AckBank => {
            if pair_bank_id(&snap, RUNE_ESSENCE_ID) >= RUNE_ESSENCE_SEED
                && snap.bank_loaded()
                && snap.bank_component_id() >= 0
                && pair_near(&snap, FALADOR_EAST_BANK, 8)
            {
                slot.stage = PairPrepStage::CloseBank;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).open_nearest_booth() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::CloseBank => {
            if snap.bank_component_id() < 0 {
                slot.stage = PairPrepStage::FirstLoad;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).close_modal() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::FirstLoad => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.first_load_sent {
                let n = match slot.kind {
                    PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                    PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                    PairCompanionKind::FlaxSpinner | PairCompanionKind::DuelPeer => 0,
                };
                cheat(c, &format!("give blankrune {n}"));
                cheat(
                    c,
                    &tele_args(
                        MULECRAFTER_AIR_RUINS.level,
                        MULECRAFTER_AIR_RUINS.x,
                        MULECRAFTER_AIR_RUINS.z,
                    ),
                );
                slot.first_load_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitReady;
        }
        PairPrepStage::WaitReady => {
            let want = match slot.kind {
                PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                PairCompanionKind::FlaxSpinner | PairCompanionKind::DuelPeer => 0,
            };
            if pair_near(&snap, MULECRAFTER_AIR_RUINS, 8)
                && pair_inv_id(&snap, RUNE_ESSENCE_ID) >= want
                && snap.bank_component_id() < 0
            {
                slot.stage = PairPrepStage::Idle;
            }
        }
        PairPrepStage::Wear => {
            if !send_ok(slot, now) {
                return;
            }
            pair_wear_first_inv(c, &snap);
            slot.last_action = now;
            slot.stage = PairPrepStage::WaitWear;
        }
        PairPrepStage::WaitWear => {
            if pair_near(&snap, DUEL_CHALLENGE, 8)
                && pair_weapon_equipped(&snap)
                && snap.modals().main < 0
            {
                slot.stage = PairPrepStage::Idle;
            } else if send_ok(slot, now) {
                pair_wear_first_inv(c, &snap);
                slot.last_action = now;
            }
        }
        PairPrepStage::Idle => {}
    }
}

fn send_ok(slot: &PairCompanionSlot, now: Instant) -> bool {
    now.duration_since(slot.last_action) >= Duration::from_millis(400)
}

fn pair_relog_seen(
    logout_sent: bool,
    saw_logout: bool,
    ingame: bool,
    logout_session: u64,
    session: u64,
) -> bool {
    saw_logout || !ingame || (logout_sent && session != logout_session)
}

fn pair_near(snap: &GameSnapshot, dest: WorldTile, radius: i32) -> bool {
    snap.tile().is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs() <= radius && (z - dest.z).abs() <= radius
    })
}

fn pair_inv_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.inv()
        .iter()
        .filter(|(item_id, _)| *item_id == id)
        .map(|(_, count)| *count)
        .sum()
}

fn pair_inv_any(snap: &GameSnapshot) -> bool {
    snap.inv().iter().any(|(_, count)| *count > 0)
}

fn pair_weapon_equipped(snap: &GameSnapshot) -> bool {
    snap.equipment()
        .iter()
        .any(|item| item.count > 0 && item.def.id > 0)
}

fn pair_wear_first_inv(c: &mut Client, snap: &GameSnapshot) {
    let Some((id, _)) = snap.inv().iter().copied().find(|(_, count)| *count > 0) else {
        return;
    };
    match Interactions::new(snap, c).wear(id) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
}

fn pair_bank_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.bank()
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}

fn pair_stat(snap: &GameSnapshot, id: i32) -> i32 {
    snap.stats()
        .iter()
        .find(|row| row.index == id)
        .map(|row| row.base)
        .unwrap_or(0)
}

fn pair_fleet_seed() -> Seed {
    Seed {
        profiles: vec![("test", "test"), ("test2", "test2")],
        mainland: true,
    }
}

fn pair_watch_settings(name: &'static str, start_script: &'static str) -> ScenarioSettings {
    ScenarioSettings {
        full_rate: true,
        only_render_selected: false,
        require_mainland_base: true,
        deadline: SCRIPT_GOLD_DEADLINE,
        start_script: Some(start_script),
        terminal_shot: Some(name),
        nav: gold_script_nav(),
        ..Default::default()
    }
}

fn pair_companion(kind: PairCompanionKind) -> Companion {
    Companion {
        profile: 1,
        per_frame: {
            let mut slot = PairCompanionSlot::new(kind);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}

/// NatureCrafter Air Master/Runner: two visible slots, shared Start after
/// both native preps. Pair watch supplies complementary mode/partner bags.
fn nature_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Air master talisman at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
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
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "nature_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::AirRunner)],
        settings: pair_watch_settings("nature_crafter_air", "NatureCrafter"),
    }
}

/// MuleCrafter Air Crafter/Mule: two visible slots, shared Start after both
/// native preps. bankFill=true is the pair_settings default, not this cell.
fn mule_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Mule crafter's exact noted essence bank before Start",
        FALADOR_EAST_BANK,
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
        "runecraft",
        1,
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the Mule crafter essence bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the Mule crafter noted essence through the bank window",
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed Mule crafter talisman and first 27 essence at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &format!("give blankrune {PAIR_MULE_FIRST_LOAD}"));
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one unnoted 1436 load of 27 before Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
            },
        ),
        (
            "confirm no extra unnoted essence beyond the first load",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
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
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "mule_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::MuleMule)],
        settings: pair_watch_settings("mule_crafter_air", "MuleCrafter"),
    }
}

/// FlaxRunner Runner/Spinner: empty packs, runner at the field, spinner at
/// the meet. First flax pack is picked after shared Start.
fn flax_runner_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty Flax runner pack at the field before Start",
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
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "flax_runner",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::FlaxSpinner)],
        settings: pair_watch_settings("flax_runner", "FlaxRunner"),
    }
}

/// DuelArena both actors: bronze scimitar at the challenge anchor, wear it,
/// then shared Start. Counterpart identity is native witness-owned; the
/// frozen script has target stats and no partner setting.
fn duel_arena_scenario() -> Scenario {
    let dest = DUEL_CHALLENGE;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Duel bronze scimitar at the challenge anchor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give bronze_scimitar 1");
                cheat(c, &tele_args(dest.level, dest.x, dest.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm bronze scimitar in pack before Start",
        Proof::Item {
            name: "Bronze scimitar",
            count: 1,
        },
    ));
    steps.push(Step {
        name: "wear the seeded bronze scimitar before Start",
        kind: StepKind::Repeat {
            send: Box::new(|c, snap| {
                pair_wear_first_inv(c, snap);
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemAtMost {
                name: "Bronze scimitar",
                count: 0,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    Scenario {
        name: "duel_arena",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::DuelPeer)],
        settings: pair_watch_settings("duel_arena", "Duel Arena Combat Trainer"),
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
#[path = "scenario_tests.rs"]
mod tests;
