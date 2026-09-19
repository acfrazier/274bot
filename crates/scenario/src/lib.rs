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

mod catalog;
pub mod evidence;
pub mod fixture;
pub mod proof;
mod render_betty_views;
mod runner;
mod scenarios;
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

pub use catalog::{get, names};
pub use evidence::{Evidence, InvRow, StatRow};
pub use fixture::{
    apply_fixture_mode, as_run_prepared, default_fixture_path, default_fixture_sav_dir,
    fixture_prereqs_of, fixture_preset_for, harness_writer_script, prepare_offline_fixture,
    run_prepared_has_setup_cheats, FixtureAccount, FixtureIdentity, FixtureMode,
    OfflinePrepareOpts,
};
pub use proof::Proof;
pub use runner::{RunnerStatus, ScenarioRunner};
pub(crate) use scenarios::script_live_seed_steps;
pub use scenarios::{nav_full_scenario, thiever_sustained_scenario};

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
    /// Scenario-only native lamp witness. Sends no game action. The runner
    /// latches one post-entry episode from its existing host hold input plus
    /// snapshot facts: hold while `lamp_id` is present, `reward_stat` XP
    /// advance, consumption, the source-identified award continuation,
    /// drainage, and hold release. This is host observation, not snapshot
    /// ownership.
    ObserveLampRedemption { lamp_id: i32, reward_stat: i32 },
    /// Observe one authentic server-owned Maze episode. The runner captures
    /// the pre-event return tile/inventory before any optional one-shot
    /// `trigger` send, then requires a held canonical spawn, ordered tile
    /// progress, the shrine region, server return/reward, and native hold
    /// release. It never sends movement, doors, or Touch.
    ObserveMazeCompletion {
        spawns: &'static [WorldTile],
        shrine: WorldTile,
        shrine_radius: i32,
        min_progress: i32,
        entry_shot: &'static str,
        /// One authentic debugproc, sent after the pre-entry baseline is
        /// latched. `macro_maze` starts itself; do not wait on the transient
        /// Old Man NPC.
        trigger: Option<&'static str>,
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
