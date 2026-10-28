//! Shared scenario layer: one scenario, two runners.
//!
//! A scenario is a seed (which profiles to log in, mainland hop), a
//! sequence of run steps (each an action send plus the evidence it must
//! produce within an observed game-tick budget), and a proof predicate over the
//! `GameSnapshot` — the same observable state scripts and the host read.
//! The headed runners (`panel-play` and `tui-play --live script_<name>`) and
//! the headless runner (`crates/e2e` under `LIVE=1`) drive the same
//! [`ScenarioRunner`] and share pass/fail behavior. The PASS/FAIL
//! contract replaces the old "5 Running polls" `LiveScript` stub: a
//! scenario proves real game state, not that a poll loop ran.
//!
//! Waiting names evidence, not sleeps (the 377 harness pattern): a step
//! sends once, then waits `budget_ticks` observed game ticks (one per
//! `PLAYER_INFO`) for a named predicate on the evolving snapshot. Each runner
//! pumps the machine from its own client-cycle hook (the panel slot thread
//! headed, the `run_with_io` hook headless), so no thread sleeps inside the
//! shared layer.

mod catalog;
pub mod evidence;
pub mod fixture;
pub mod proof;
mod quest_fast;
pub mod quester;
pub mod random_episode;
mod render_betty_views;
mod runner;
mod scenarios;
pub mod shot;
pub mod tutorial;

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

pub use catalog::{get, is_registered, names, try_get};
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
pub use scenarios::{
    duel_arena_completed_duel_step, nav_full_scenario, thiever_sustained_scenario,
};
pub use scenarios::{quester_stage, quester_stage_with_watch};

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
    /// A world-wide engine speed restored after this scenario reaches a
    /// terminal state. Live quest-fast cells set this only on Engine Q.
    pub teardown_world_speed_ms: Option<u32>,
    pub require_mainland_base: bool,
    /// Background cheats the runner fires while the scenario is running
    /// (377 sustain: energy, HP, stats). Empty for most scenarios.
    pub sustains: Vec<Sustain>,
    /// A picker card the host **selects** when the live runner is installed,
    /// then Starts on [`StepKind::StartScript`] after the last seed wait —
    /// the scenario's later steps observe the running script's evidence.
    /// Catalog / TradeBot fixture names load JS; a compiled registry id
    /// (currently `Sherlock`) starts that port. `None` for host-driven
    /// scenarios.
    pub start_script: Option<&'static str>,
    /// A heterogeneous or multi-slot catalog launch owned by the live
    /// harness. Unlike `inject_companion_as`, this can select a different
    /// card per slot and can start all four members of a party.
    pub fleet_start: Option<FleetStart>,
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
    /// Owned script values for scenarios selected from runtime input. These
    /// merge after the static catalog defaults above.
    pub script_settings_overrides: Option<Map<String, Value>>,
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
    /// Fail the run when the driven player dies after StartScript: the
    /// content sends `Oh dear you are dead!` on every ordinary death
    /// (`player/scripts/death.rs2:27`). A cell whose later watches a
    /// respawn, restock and return could also satisfy sets this, so a death
    /// and a recovery cannot stand in for the intended trip.
    pub fail_on_death: bool,
}
/// Harness-owned multi-slot catalog launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetStart {
    /// Slot 0 runs ClueSolver; slot 1 runs the clue-mode Duel helper.
    ClueDuel,
    /// Every profile runs JiveKQ with one shared minted roster.
    JiveKq,
}

/// How long the JiveKQ leader's Start waits after its three peers started,
/// so they first prove their missing-peer bank hold.
pub const JIVE_KQ_LEADER_DELAY: Duration = Duration::from_secs(30);

/// One Start of a [`FleetStart`]: the catalog card a front end loads on the
/// minted slot `names[slot]`, the harness settings merged last into its
/// bag, and its hold after the members stashed before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetMember {
    pub slot: usize,
    pub card: &'static str,
    pub settings: Vec<(&'static str, String)>,
    pub delay_after_peers: Option<Duration>,
}

impl FleetStart {
    /// The launch plan for the run's minted `names` (driven slot first), in
    /// Start order. Partner and roster settings name players as the game
    /// shows them (`JString::to_screen_name`), as the frozen cards compare.
    pub fn members(self, names: &[String]) -> Result<Vec<FleetMember>, String> {
        let shown = |name: &String| client::util::JString::to_screen_name(name);
        match self {
            Self::ClueDuel => {
                let [solver, helper] = names else {
                    return Err("ClueDuel fleet requires exactly two profiles".into());
                };
                Ok(vec![
                    FleetMember {
                        slot: 0,
                        card: "ClueSolver",
                        settings: vec![("clueDuelPartner", shown(helper))],
                        delay_after_peers: None,
                    },
                    FleetMember {
                        slot: 1,
                        card: "Duel Arena Combat Trainer",
                        settings: vec![
                            ("mode", "Clue helper".to_string()),
                            ("partner", shown(solver)),
                        ],
                        delay_after_peers: None,
                    },
                ])
            }
            Self::JiveKq => {
                if names.len() != 4 {
                    return Err("JiveKq fleet requires exactly four profiles".into());
                }
                let roster = names.iter().map(shown).collect::<Vec<_>>().join(",");
                // The driven leader starts last.
                Ok([1, 2, 3, 0]
                    .into_iter()
                    .map(|slot| FleetMember {
                        slot,
                        card: "JiveKQ",
                        settings: vec![("team", roster.clone())],
                        delay_after_peers: (slot == 0).then_some(JIVE_KQ_LEADER_DELAY),
                    })
                    .collect())
            }
        }
    }
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
    /// Let the runner's own Walk/Follow routes cross danger zones, as the
    /// **Danger routing: Always** Nav setting does for a user's walks
    /// (`ZoneExempt::all()`). Default false keeps the product default.
    pub allow_danger_zones: bool,
    pub show_nav_path: bool,
    pub hop_labels: bool,
    pub hop_label_px: i32,
    pub collision_fill: bool,
    pub nsew_labels: bool,
    pub client_trail: bool,
    pub component_flood: bool,
    pub camera_follow: bool,
    /// Explicit world-speed request (`::speed N`) once the seed releases.
    /// `None` — the default — sends nothing and keeps the engine at its
    /// default game-tick duration. A request is always sent; the runner
    /// confirms it with a system-chat line newer than that send.
    pub engine_speed_ms: Option<u32>,
}

impl Default for ScenarioNav {
    fn default() -> Self {
        Self {
            allow_teleports: false,
            allow_wilderness: false,
            allow_danger_zones: false,
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
/// a non-default game-tick duration is opt-in via [`ScenarioNav::with_tick_ms`].
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
        allow_danger_zones: false,
        engine_speed_ms: None,
    }
}

impl ScenarioNav {
    /// Request the world-wide game-tick duration, in milliseconds. The runner
    /// sends this request when the seed releases.
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
    /// Fire every running client cycle while `proof` holds (e.g. energy ≤ 25).
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
            teardown_world_speed_ms: None,
            require_mainland_base: false,
            sustains: Vec::new(),
            start_script: None,
            fleet_start: None,
            start_file: None,
            wait_script_stop: None,
            script_settings_inject: None,
            script_settings_overrides: None,
            inject_companion_as: None,
            fixture_prereqs: None,
            fixture_loadouts: None,
            fail_on_death: false,
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
    /// then wait for `wait.arm` within its observed game-tick budget.
    Perform {
        #[allow(clippy::type_complexity)]
        send: Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>,
    },
    /// Nav walk: arm `nav_planner::router::find` over the collision + transport
    /// graph derived from the baked pack and drive `Traveller::follow`
    /// one route step per client cycle until arrival. Identical to `Follow`;
    /// arm is `arrived(dest)`, the proof mirrors it, so a walk that ends
    /// anywhere but the destination fails the step with the terminal
    /// outcome's message.
    Walk { dest: WorldTile },
    /// Whole-world nav: arm `nav_planner::router::find` over the collision +
    /// transport graph derived from the baked pack and drive
    /// `Traveller::follow` one route step per client cycle until the route terminates.
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
    /// Dialog janitor: drain the seed's dialogs **every client cycle** until
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
    /// An explicit [`Proof::LoggedOut`] arm completes on the observed
    /// off-world frame; every other arm also requires the new online session.
    Relog,
    /// Like [`StepKind::Perform`], but re-sends once per observed game
    /// tick (one `PLAYER_INFO`) until `wait.arm` holds (`getvar` polls,
    /// sticky `setvar tutorial`). Client cycles do not re-send it.
    Repeat {
        #[allow(clippy::type_complexity)]
        send: Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>,
    },
    /// Native seed deposit through the open bank window. Pending bank-side
    /// data re-arms once per observed game tick. An accepted send waits for
    /// `wait.arm` without sending again, for at most two observed game ticks.
    /// The runner owns the attempt so missing rows, missing Deposit All
    /// operations and refused sends name their branch in the failure.
    NativeSeedDeposit {
        unnoted_id: i32,
        quantity: i32,
        note_id: Option<i32>,
    },
    /// Host-only fleet observation. Sends no game action and advances only
    /// when `ready` accepts the evolving driven-slot snapshot. Companion
    /// closures may feed the same shared witness. `evidence` is the named
    /// receipt emitted by headed and headless runners.
    Await {
        evidence: &'static str,
        #[allow(clippy::type_complexity)]
        ready: Box<dyn Fn(&GameSnapshot) -> bool + Send + Sync>,
        /// Read-only terminal summary of what this step is still
        /// waiting for, plus any observation values it reports.
        /// The runner appends the most recent reached summary to
        /// the failure message (a step tick budget, the whole-run
        /// deadline, or a later step), and records it as the pass
        /// receipt's message. A failure receipt therefore names
        /// the unmet witness terms instead of only the step, and a
        /// pass receipt still records observation values, such as
        /// a chance outcome that must never gate the run. It sends
        /// nothing and must not change witness or game state.
        /// `None` keeps the plain messages.
        summary: Option<Box<dyn Fn() -> String + Send + Sync>>,
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
    /// Observe combat XP across at most two stall sessions. After a catch-less
    /// first bank visit, require a closed bank, return to `stand`, and renewed
    /// native `guard_ready` evidence. A second unqualified bank visit fails.
    /// Sends no actions and preserves the Start-anchored cumulative XP arm.
    ObserveStallCombat {
        stand: WorldTile,
        guard_ready: Proof,
    },
    /// Observe fresh XP after a bank return. A fresh bank visit starts a
    /// bounded return leg; its closed-bank arrival at `stand` restarts the
    /// XP baseline and the same game-tick budget. At most `max_restarts`
    /// further bank returns may restart the watch. Each bank/return phase
    /// has the same finite budget. Sends no actions.
    ObserveCombatAcrossBankReturns {
        stand: WorldTile,
        radius: i32,
        max_restarts: u8,
    },
    /// Wait for the card's selected combat style (`wait.arm`, a varp proof)
    /// across the Loot-count bank trips that may come before it. A card that
    /// selects its style only out of combat, after its restock and its bank
    /// task, can first fight a catch or loot a public drop; each such item
    /// sends it on a whole bank trip. Two content progress signals restart
    /// the step's game-tick budget: a fresh open bank that shows more
    /// `deposit_id` than this step has seen (one completed trip), and an XP
    /// gain in `progress_stat` (one successful restock action). The budget so
    /// bounds one leg between progress signals, not the number of trips.
    /// Melee XP (Attack, Strength or Defence) that rises while the pack holds
    /// an item in `food` and the arm does not hold fails at once: the card
    /// fought without its selected style although it was free to select it.
    /// Sends no actions.
    ObserveStyleAcrossLootTrips {
        deposit_id: i32,
        food: &'static [i32],
        progress_stat: i32,
    },
    /// Summon one random event through the engine's own random-event
    /// machinery and judge the host Guardian's handling of it
    /// ([`random_episode`]). The runner sends only the cell's trigger (and
    /// re-sends it on a fresh busy reply); everything else is observed from
    /// the host hold and the snapshot. A trigger that needs a missing engine
    /// test patch ends the run as UNAVAILABLE, not FAIL.
    ObserveRandomEvent {
        cell: random_episode::RandomEventCell,
    },
    /// Host starts the catalog isolate (`script_start_load`) or compiled
    /// card (`start_compiled`) once when the live pump sees this step.
    /// No-op on the client. File cards wait a one-game-tick arm; compiled
    /// cards wait until Start is Running (or fail with the rejection).
    StartScript,
    /// Host stops the selected script and its walk. No-op on the client.
    StopScript,
}

/// Evidence wait for a step: a named predicate and a game-tick budget.
pub struct Wait {
    /// The named evidence arm the wait ends on.
    pub arm: Proof,
    /// Observed game ticks the wait may take: one per `PLAYER_INFO`
    /// (the server's world cycle, about 600 ms on 289). Client cycles
    /// (frontend polls) and dirty snapshots do not count.
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
    /// Extra profile slots in the scenario fleet. Their `per_frame` hooks
    /// run once per client cycle. Profile 0 drives scenario steps. Each
    /// companion uses another `seed.profiles` index (`>= 1`). The runner
    /// invokes [`ScenarioRunner::companion_tick`] once per client cycle, like
    /// the driven slot's client-cycle hook. Empty for single-bot scenarios.
    pub companions: Vec<Companion>,
    /// Boot settings for the scenario: view knobs for the headed panel and
    /// runner-consumed deadline / terminal shot / mainland-base gate.
    pub settings: ScenarioSettings,
}

/// One extra profile slot in a scenario fleet. `profile` is an index into
/// `seed.profiles` (0 is the driven slot, so companions are `>= 1`). The
/// `per_frame` hook runs once per client cycle, like the driven slot's hook.
pub struct Companion {
    pub profile: usize,
    pub per_frame: Box<dyn FnMut(&mut Client) + Send>,
}

/// Gold script cells run on the engine's documented 600 ms default game tick
/// (`World.TICKRATE`) and request no speed change. `::speed` is world-wide,
/// so the runner sends it only for a non-default game-tick duration a caller
/// explicitly asks for (`ScenarioNav::with_tick_ms`, `QUESTER_TICK_MS` on Engine Q).
fn gold_script_nav() -> ScenarioNav {
    ScenarioNav::default()
}

/// Ordinary post-Start watch budget in **observed game ticks** (one per
/// `PLAYER_INFO`; see [`Wait`]). The number was sized from live runs
/// under the old dirty-snapshot counting. The same number now bounds
/// the wait by game time instead of by snapshot churn.
const SCRIPT_GOLD_WATCH_TICKS: u32 = 150;
/// Brimhaven agility-arena watch for the first ticket after the first
/// Tag, in observed game ticks (one per `PLAYER_INFO`; see [`Wait`]).
///
/// Content bound: the arena clerk moves the flashing arrow to a new
/// pillar every 98 `map_clock` ticks (`agilityarena_clerk.rs2:78-86`
/// sets the next pillar time to `map_clock + 98`). The player pillar
/// timer fires at the end of a period and keeps ticket eligibility
/// only when a pillar was tagged in that period
/// (`agilityarena.rs2:40-48`). The first tag sets eligibility and
/// grants no ticket; a tag in the next period grants the ticket
/// (`agilityarena.rs2:67-84`). A first tag just after one period
/// begins and a second tag just before the next period ends can
/// therefore be up to 2 * 98 = 196 game ticks apart. This watch
/// starts after the first tag, so the bound from its own start is
/// smaller. The value 220 adds a stated 24-game-tick margin over
/// the 196-tick bound for timer re-arm and observed-tick alignment.
/// The ordinary gold watch and the 180 s [`SCRIPT_GOLD_DEADLINE`]
/// stay unchanged; the deadline can still end a worst-case run
/// first. Only the `brimhaven_agility` ticket step uses this budget.
const BRIMHAVEN_TICKET_WATCH_TICKS: u32 = 220;
/// Seed + tele + drain + the short watch. Not a 6-minute soak. Wall-clock
/// whole-scenario cap (independent of game-tick budgets).
const SCRIPT_GOLD_DEADLINE: Duration = Duration::from_secs(180);
/// FlaxRunner pair cell whole-run cap. The worst of three measured passing
/// runs took 170.6 s in total: 155.5 s of script plus 15.1 s of setup
/// (`FIX-025/f/flax/report.md`). That is only 9.4 s under the shared 180 s
/// [`SCRIPT_GOLD_DEADLINE`], and the nightly run was still spinning flax
/// when that cap ended it. 240 s is about 1.4x the worst measured total,
/// a 69.4 s margin. Only `flax_runner` uses this cap; the other pair
/// cells keep [`SCRIPT_GOLD_DEADLINE`].
const FLAX_RUNNER_DEADLINE: Duration = Duration::from_secs(240);
/// BrimhavenAgility whole-run cap. A valid run can need about 318 counted
/// game ticks: up to 122 ticks outside the ticket watch (`run-1`, 272 ticks
/// in 179.6 s with the old 150-tick watch) plus the 196-tick content bound
/// on a late second tag ([`BRIMHAVEN_TICKET_WATCH_TICKS`]). At 600 ms that
/// is about 191 s, plus 11.7-16.4 s measured outside counted ticks, so about
/// 207 s; the shared 180 s [`SCRIPT_GOLD_DEADLINE`] would end such a run
/// (`NIGHTLY-TRIAGE-1024/brimhaven/review-opus-r2.md`, round 3, R3-1).
/// 240 s allows about 372 ticks. Only `brimhaven_agility` uses this cap.
const BRIMHAVEN_AGILITY_DEADLINE: Duration = Duration::from_secs(240);
/// SmithingBot product→fresh-bank watch after the first pack item appears.
///
/// **Units:** this constant is a `budget_ticks` count of observed game
/// ticks (one per `PLAYER_INFO`; see [`Wait`]). It is not wall time.
/// The sizing below was measured under the old dirty-snapshot counting:
/// live fail at cf41 had outcome total_ticks 204 (dirty counts then) /
/// 102.075s wall with snapshot `tick` 173 (game ticks). Those figures
/// remain the sizing history.
///
/// **Content game-tick lower bound (selected289 `smithing_anvil`):**
/// `p_delay(1)` + work + `p_delay(2)`; `P_DELAY` → `delayedUntil =
/// currentTick + 1 + n` → 2+3 = **5 game ticks/bar**. `makeFromPanelMax`
/// → Make-10 (op 3). Hammer kept → ≤27 bars/trip; deposit arms on first
/// product → ≤26 remain → 26×5 = **130 game ticks** pure forge.
/// Bronze dagger 12.5 xp; RS thresholds 83 / 174 / 276 → level 1→2, 2→3,
/// and **3→4** all land inside a 27-bar first trip (3→4 still in the
/// remaining ≤7 bars after measured 20 daggers / 250 xp / level 3). Each
/// level-up plus a Make-10 residual restart needs ContinueDialog + re-Use
/// anvil + panel (~15 game ticks × 4 interrupts ≈ 60 game ticks). Anvil
/// (3188,3425) → VW bank Chebyshev 15 + open/deposit ≈ 30 game ticks.
/// Game-tick lower bound ≈ 130+60+30 = **220 game ticks** (not assigned raw
/// to this constant).
///
/// **Prior dirty-counter sizing:** the measured deposit step exhausted the
/// old 150-dirty-count limit mid-craft (20×1205 + 7 bars, anim 898, three
/// MakePanel op3 + ContinueDialog pairs, no deposit) while the craft progressed.
/// The old estimate used about 8 dirty snapshots/item for 7 bars, the L3→4
/// level-up and the bank walk. It assigned 240 to the old dirty counter.
/// This constant now means 240 observed game ticks (144s at 600ms per game tick).
/// The unchanged 180s [`SCRIPT_GOLD_DEADLINE`] can end the run first.
/// Root LIVE must confirm deposit/restock/return/fresh XP within that deadline.
/// The global deadline and other gold game-tick watches stay unchanged.
const SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS: u32 = 240;
/// HerbloreSecondaries eggs: first-Take → fresh bank 223, then dungeon return.
///
/// **Units:** `budget_ticks` are observed game ticks (one per
/// `PLAYER_INFO`; see [`Wait`]). They are not wall seconds. The sizing
/// below was measured under the old dirty-snapshot counting.
/// live7wnm9z_0: outcome 170 dirties / 82.065s wall / snapshot `tick`
/// 141 (game ticks). Those figures remain the sizing history.
///
/// **Old dirty-counter observation:** step 29 `fresh_bank_item_id(223)>=1`
/// exhausted the 150-dirty-count limit while the player remained in the field.
/// Terminal 3117,9951, inv 6×223 + 6×379, bank closed gen 6, paint
/// Got 6 / Trips 0 / Food 6, status taking eggs, in combat HP 6/10.
/// Canonical `needsRestock` is false until packFull or
/// (`takeFood && foodWant>0 && foodCount<1`). foodWithdraw default 10 is
/// already pinned; 6 lobster remain. Collection was progressing: six
/// named Takes plus four Eat, then a respawn Take on the first tile.
/// This observation does not rule out defects later in the cycle.
///
/// **Respawn (game ticks — justification only):** `red_spiders_eggs.obj`
/// has no `respawnrate` (snape_grass sets 30). `ObjType.respawnrate = 100`
/// default "1-minute". `OBJ_TAKEITEM` on `EntityLifeCycle.RESPAWN` calls
/// `World.removeObj(obj, objType.respawnrate)`; `scaleByPlayerCount(100)`
/// with 1 player is 99 game ticks. First Take 3117,9951 then the same
/// tile at FAIL; capture game tick 141 matches ~100 game ticks after first
/// take. Six distinct in-radius Takes before that respawn. Packed
/// `o48_155` obj list is not in this checkout; count is the measured 6.
///
/// **Provisional arm-to-deposit estimate:** remaining 6 lobster, eat at HP≤5 (lobster heal
/// 12 never fits maxHp 10). Measured 4 eats during the ~60s first-respawn
/// wait ≈ 15s/eat → ~90s more. Then Edgeville dungeon walk: field
/// 3120,9952 → ladder 3096,9868 Chebyshev 84 + surface ~25 + booth/deposit
/// ≈ 80s one-way. Watch already used ~60s. Arm-to-deposit ≈ 60+90+80 =
/// **~230s wall**. The old 150-dirty-count observation took about 60s, or
/// 2.5 dirty snapshots/s; 575 old-counter snapshots informed the existing
/// 600-game-tick (360s) deposit budget.
/// The reverse dungeon wait remains 240 game ticks (144s). First-egg,
/// empty, close and further-egg waits remain 150 game ticks.
///
/// **Deadline:** the text above sizes the cycle for the old fresh-account
/// seed (HP 10), which FIX-025-F-HERBLORE replaced with upstream's level-40
/// account. At 40 the script spends its lobsters on pack-full eats instead
/// of HP eats, so a trip gathers 26 to 27 eggs and the measured full cycle
/// grew: four completed sizing runs took 627, 622, 627 and 627 game ticks
/// (longest wall 403.05s), and a fifth had looted 26 eggs, banked and
/// returned but was still short of the terminal egg at 656 game ticks when
/// the old 420s wall expired (420.73s). The fixed deadline is 600s: 196.95s
/// above the longest complete run and 179.27s above the expired run. The
/// per-watch budgets can still sum past it (deposit 360s, empty and close
/// 90s each, return 144s), so on a stall the whole-run deadline ends the
/// scenario first. Script `walkTo` `timeoutMs` 180_000 per hop is unchanged. No
/// forced post-Start bank, debug command or engine acceleration was used.
/// The prior 1755 ladder stall (empty-pack cell) is not included.
const HERBLORE_EGG_DEPOSIT_WATCH_TICKS: u32 = 600;
const HERBLORE_EGG_RETURN_WATCH_TICKS: u32 = 240;
const HERBLORE_EGG_DEADLINE: Duration = Duration::from_secs(600);
/// B2's completed HerbloreSecondaries cycle needed 412 observed game ticks
/// (`FIX-025/integrate-b2/engine-a-600ms-b2/cells/herblore_secondaries/receipt.json`).
/// The newt branch still returned from Draynor at its old 180s deadline.
/// Use the completed cycle as a conservative bound: 412 ticks × 600ms,
/// plus 50% headroom for preparation and scene settling (618 ticks, 370.8s).
/// Per-step newt watches remain 150 game ticks.
const HERBLORE_NEWT_DEADLINE: Duration = Duration::from_millis(412 * 600 * 3 / 2);
/// ArdyCakes first cake → Ardougne bank arrival: the Baker stall fill of the
/// 6 free slots (22 Knife ballast) plus the bank walk.
///
/// **Owner refusals (289 content).** The Baker (npc 571 `baker_merchant`)
/// spawns at (2669,3310) (m41_51 local 45,46), which is the stall's
/// alternate stand. He has the default `wanderrange` 5 (NpcType.ts), and
/// Npc.ts `wanderMode` re-picks a random tile within ±5 of spawn with 1/8
/// chance per game tick. `stealing_check_for_owner` (stealing.rs2) refuses a
/// steal silently (overhead text only) when he is within 5 tiles with line
/// of sight. Both frozen stands lie inside his wander box: with the 2×2
/// stall as the only blocker, he sees (2668,3312) from 50% of it and
/// (2669,3310) from 68%. A simulation of that wander with the frozen loop
/// gives 0.63 refusals per attempt in steady state. The two live ardy_cakes
/// runs show 29 of 44 attempts refused (0.66): 5/12 at 290253afd and 24/32
/// at 0c4e035a7. An earlier sizing pooled 5/19 from the other Ardougne
/// cells and was too optimistic.
///
/// **Engine model (frozen stealCakes):** after the first cake five more
/// are needed. A success is followed by the stall's `respawn_ticks` 8
/// (stealing.dbrow). A refusal costs the 2.4 s resolve (~4 game ticks), and
/// three in a row swap stands (~2 game ticks). A guard catch (0.12 per attempt)
/// costs the Flee kite, the combat bar, the 10-game-tick lockout and the walk
/// back (measured 32 game ticks, live 290253afd). The bank walk is ~25 game
/// ticks. A 20k-run simulation of the Baker's wander and this loop gives a
/// median of 9 refusals before the sixth cake (p90 21, p99 35), and P(arrival
/// within 209 game ticks) = 0.78. 95% needs 305 game ticks (p99 395).
/// This model puts the 0c4e035a7 run (fill ~245 game ticks + walk, 24
/// refusals) at about the 90th percentile of arrival time.
///
/// **Budget:** A 20k-run content model estimates a 95th-percentile first
/// cake fill and bank arrival at 305 game ticks (p99 395). The watch allows
/// 320 game ticks, 15 ticks above the p95 estimate (192 seconds at 600 ms
/// per game tick). Other ArdyCakes waits remain 150 game ticks. The further
/// cake after return needs one stall success: P(≤143 game ticks) = 0.96
/// under the same model. The 300-second whole-run deadline can pre-empt this
/// watch.
const ARDY_CAKES_DEADLINE: Duration = Duration::from_secs(300);
/// ArdyThiever Fight whole-run cap. After Start the cell runs an opening
/// Baker stall session until a Guard catches, the FightBack kill (27 game
/// ticks measured on `ardy_cakes_fight`, 290253afd), another stall visit if
/// the catch came before any food (10-game-tick lockout first), then pickpockets
/// until coins (126/256 per attempt, ~11 game ticks per stunned failure), the
/// bank deposit and return (~28 + ~25 game ticks measured on the live Fight
/// run), and the pickpockets until further coins. A 200k-run Monte Carlo of
/// that chain, over runs whose catch lands inside the Strength watch, gives a
/// 95th percentile of 240/256/247 game ticks at the pooled / ArdyCakes-only
/// / `ardy_cakes_fight` stall rates, and a 99th of 278/291/330. With ~25 s
/// of seed and Start at 0.6 s per game tick, the 95th percentile is 169-179 s,
/// at the 180 s gold cap, and the 99th is 192-223 s. Use **240 s**.
const ARDY_THIEVER_FIGHT_DEADLINE: Duration = Duration::from_secs(240);

/// GnomeMagicChopper fletch cells: the first Magic-tree Woodcutting XP arm and
/// the further Magic logs arm after the deposit return.
///
/// **Units:** `budget_ticks` are observed game ticks (one per
/// `PLAYER_INFO`; see [`Wait`]). They are not wall time. The sizing
/// below was measured under the old dirty-snapshot counting:
/// `gnome_fletch_short` at `290253af` failed step 17 after 150 dirties,
/// about 120 game ticks, so about 1.25 dirty snapshots per game tick.
///
/// **Engine odds (selected289 `skill_woodcutting`, justification only):**
/// Magic tree with a Rune axe has `successchance` 7,21. At Woodcutting 75,
/// `stat_random` gives floor(7*24/98)+floor(21*74/98)+1 = 17, so a roll
/// succeeds with p = 17/256 (about 6.6%). `%action_delay = clock+3` and the
/// roll lands on `%action_delay = clock`, so the first roll is 3 game ticks
/// after the op, then one roll every 4 game ticks. The mean wait for the first
/// log is about 59 game ticks. The 99th percentile is 68 rolls, about
/// 271 game ticks.
///
/// **Why 150 is wrong for this arm:** the first arm also covers the
/// card's own gear bank trip (upstairs booth and back), about 50 game ticks.
/// That left about 68 game ticks, or 17 rolls, of steady chopping.
/// There was one click, one swing message, rune-axe animation 867 held and
/// no re-click. With a correct product the chance of no log in 17 rolls is
/// (239/256)^17, about 31%.
///
/// **Budget:** under the old dirty counting, 450 dirties was about 360
/// game ticks. Take off about 53 game ticks for the gear trip and the
/// first-roll delay. That leaves about 77 rolls, so P(no log) is about 0.5%.
/// A 450-game-tick budget keeps at least that margin. The further-log
/// arm covers the stair walk (or a walk to another Magic tree after the
/// 1/8 depletion) plus the same roll tail. Other arms keep 150.
///
/// **Deadline:** pre-Start about 12s. Gear trip about 30s. Fletch about 5s.
/// Deposit trip and return about 35s. Two geometric chops at 2.4s a roll.
/// 420s leaves about 140 rolls for two logs, so P(fewer than 2) < 0.1%.
/// The per-arm budgets are the binding limit.
const MAGIC_TREE_CHOP_WATCH_GAME_TICKS: u32 = 450;
const MAGIC_TREE_CHOP_DEADLINE: Duration = Duration::from_secs(420);

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

/// Where `api::interact::mainland_hop` lands (`tele 0,50,50,20,20`). A
/// mainland seed releases only on this exact tile, so it is also the Start
/// tile of a mainland-seeded File card whose seed does not move it.
pub(crate) const MAINLAND_LANDING: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

/// Catalog Start after the last seed wait: live pumps call `script_start_load`
/// once, then this one-game-tick arm (run energy, not XP) succeeds.
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

/// Game-tick budget for a compiled Start to reach Running. Preparation is
/// off-pump and usually settles in milliseconds. This is a miss detector,
/// not a soak.
const COMPILED_START_GAME_TICKS: u32 = 50;

/// Compiled registry Start after the last seed wait: live pumps call
/// `start_compiled` once, then this arm waits until Running. A Rejected
/// or Failed Start fails the step with that reason instead of waiting.
fn start_compiled_step() -> Step {
    Step {
        name: "start the compiled card",
        kind: StepKind::StartScript,
        wait: Wait {
            arm: Proof::ScriptRunning,
            budget_ticks: COMPILED_START_GAME_TICKS,
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

/// Progress of one native seed-deposit attempt. Pending data re-arms the
/// attempt. An accepted send latches the step until its bank proof arrives
/// or the step's existing observed-game-tick budget expires.
///
/// Deposit All moves the item with `inv_moveitem_uncert`, without `p_delay`
/// (`interface_bank/scripts/bank_deposit.rs2:4,18-27`, `bank.rs2:87-107`,
/// move at `bank.rs2:102`). The bank inventory change is the proof signal,
/// not a separate post-send timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SeedDepositProgress {
    /// The fresh bank already holds the seed and the pack no longer
    /// does. The attempt sent nothing.
    AlreadyDeposited,
    /// Bank-side data is not ready yet, or the dispatch raced the
    /// client's live state. The attempt sent no deposit.
    BankPending,
    /// The attempt sent the Deposit All interaction for the seed row.
    DepositSent,
}

/// The branch of a native seed-deposit attempt that failed after the
/// bank reported ready. The runner puts this text in the step
/// failure, so the receipt names the branch instead of a bare
/// driver rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SeedDepositFailure {
    /// No bank-side row matches the seed's inventory id.
    MissingRow { inv_id: i32 },
    /// A matching bank-side row exists, but none carries Deposit All.
    MissingOperation { inv_id: i32 },
    /// The Deposit All interaction was refused before dispatch.
    SendRefused { inv_id: i32, reason: SendReason },
}

impl std::fmt::Display for SeedDepositFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeedDepositFailure::MissingRow { inv_id } => write!(
                f,
                "native seed deposit: no bank-side row for item {inv_id} after bank-side readiness"
            ),
            SeedDepositFailure::MissingOperation { inv_id } => write!(
                f,
                "native seed deposit: bank-side row for item {inv_id} has no Deposit All operation"
            ),
            SeedDepositFailure::SendRefused { inv_id, reason } => write!(
                f,
                "native seed deposit: Deposit All send for item {inv_id} refused: {reason:?}"
            ),
        }
    }
}

/// True while the deposit pane cannot yet be judged: the bank is
/// closed or its main table has no current full, the side root is
/// down, or no packet for the side component has been observed.
/// These are the signals the native snapshot view uses to decide
/// that bank-side rows are posted (open withdraw component, raised
/// side root, side packet generation), plus the main table's own
/// loaded flag. A pending bank is never accepted as ready and never
/// hard-fails the deposit. The step's game-tick budget bounds the
/// wait.
fn native_seed_bank_side_pending(snapshot: &GameSnapshot) -> bool {
    snapshot.bank_component_id() < 0
        || !snapshot.bank_loaded()
        || snapshot.modals().side < 0
        || snapshot.bank_side_packet_generation().is_none()
}

/// One native seed-deposit attempt against the open bank window.
/// Pending bank-side data returns [`SeedDepositProgress::BankPending`]
/// and sends nothing. Once the readiness signals report complete
/// data, a missing row, a missing Deposit All operation, or a
/// refused send is a real failure and names its branch. The deposit
/// itself keeps the exact-id row match: bulk fixtures sit in the
/// bank-side pack as certificates, not as unnoted bases.
pub(crate) fn native_seed_deposit_attempt(
    c: &mut Client,
    snapshot: &GameSnapshot,
    unnoted_id: i32,
    quantity: i32,
    note_id: Option<i32>,
) -> Result<SeedDepositProgress, SeedDepositFailure> {
    let inv_id = note_id.unwrap_or(unnoted_id);
    if (Proof::BankItemId {
        id: unnoted_id,
        count: quantity,
    })
    .check(snapshot, None)
        && (Proof::ItemIdAtMost {
            id: inv_id,
            count: 0,
        })
        .check(snapshot, None)
    {
        return Ok(SeedDepositProgress::AlreadyDeposited);
    }
    if native_seed_bank_side_pending(snapshot) {
        return Ok(SeedDepositProgress::BankPending);
    }
    let mut rows = snapshot.bank_side().iter().filter(|item| {
        item.def.id == inv_id
            && match note_id {
                Some(_) => item.def.noted && item.def.certificate_link == unnoted_id,
                None => !item.def.noted,
            }
    });
    let Some(first) = rows.next() else {
        return Err(SeedDepositFailure::MissingRow { inv_id });
    };
    let Some((item, op)) = std::iter::once(first)
        .chain(rows)
        .find_map(|item| bank_deposit_all_op(&item.actions).map(|op| (item, op)))
    else {
        return Err(SeedDepositFailure::MissingOperation { inv_id });
    };
    let mut ix = Interactions::new(snapshot, c);
    match ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)) {
        SendResult::Sent { .. } => Ok(SeedDepositProgress::DepositSent),
        // The snapshot raced the client's live state between rebuild
        // and dispatch. Retry on the next observed game tick. The
        // shop-banker opener treats the same reasons as transient.
        SendResult::Refused {
            reason: SendReason::SceneUnavailable | SendReason::OffScene | SendReason::StaleTarget,
            ..
        } => Ok(SeedDepositProgress::BankPending),
        SendResult::Refused { reason, .. } => {
            Err(SeedDepositFailure::SendRefused { inv_id, reason })
        }
    }
}

fn native_bank_deposit(name: &'static str, seeds: Vec<NativeSeed>) -> Vec<Step> {
    seeds
        .into_iter()
        .map(|seed| Step {
            name,
            kind: StepKind::NativeSeedDeposit {
                unnoted_id: seed.unnoted_id,
                quantity: seed.quantity,
                note_id: seed.note_id,
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

/// A client cache directory for tests that holds no jag files on any
/// machine: this crate's own source tree. `/tmp` is not one; a host may keep
/// a client cache there (the Windows test host has one in `C:\tmp`), and a
/// synthetic client would then load its interfaces and configs.
#[cfg(test)]
pub(crate) const NO_CACHE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

#[cfg(test)]
#[path = "scenario_tests.rs"]
mod tests;
