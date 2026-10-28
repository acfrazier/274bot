//! The shared scenario runner: a state machine both frontends pump once
//! per client cycle. It does not sleep. Each client-cycle observation
//! reads the same client state, whether the panel thread or the headless
//! `run_with_io` hook calls it. Both runners share seed, step and proof logic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::{cheat, logout, Interactions, SendResult};
use api::obj_names::ObjNames;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav_executor::{TravelOptions, TravelOutcome, Traveller};
use nav_planner::router::{FindOptions, Route};
use nav_planner::world::NavWorld;

use crate::evidence::{Evidence, GroundGuardEvidence, GroundGuardItem, SeedDepositEvidence};
use crate::proof::Proof;
use crate::random_episode::{Frame, RandomEventEpisode, Verdict};
#[cfg(test)]
use crate::Wait;
use crate::{Scenario, StepKind, SustainWhen};

/// The runner's pollable status (the UI and the headless test read it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerStatus {
    /// Waiting for `ingame && scene_state == 2` on a mainland build base.
    Seeding,
    /// Executing run step `step` of `total` (or proving when
    /// `step == total`).
    Running { step: usize, total: usize },
    /// The proof predicate held; evidence recorded.
    Passed,
    /// A step, proof, or the deadline failed; evidence recorded.
    Failed(String),
}

enum Phase {
    Seeding,
    Running,
    Proving,
    Cleaning,
    Done,
}

enum TerminalOutcome {
    Pass,
    Fail(String),
    Unavailable(String),
}

struct TerminalCleanup {
    speed_ms: u32,
    baseline: Option<i32>,
    started: Instant,
    outcome: TerminalOutcome,
}

/// Facts retained only while one [`StepKind::ObserveLampRedemption`] is
/// current. Every latch begins after the fixture has observed the injected
/// item, so pre-injection holds/dialogues cannot satisfy the episode.
#[derive(Debug, Default)]
struct LampEpisodeObservation {
    reward_baseline: Option<i32>,
    lamp_seen: bool,
    clear_before_dialogue: bool,
    hold_seen: bool,
    reward_seen: bool,
    consumed_seen: bool,
    dialogue_seen: bool,
    dialogue_drained: bool,
    released: bool,
}

impl LampEpisodeObservation {
    fn summary(&self) -> String {
        format!(
            "lamp redemption episode [lamp={},hold={},reward={},consumed={},dialogue={},drained={},released={}]",
            self.lamp_seen,
            self.hold_seen,
            self.reward_seen,
            self.consumed_seen,
            self.dialogue_seen,
            self.dialogue_drained,
            self.released,
        )
    }
}

/// Ordered facts for one server-owned Maze episode. The fixture sends no
/// Maze actions; it only observes the shared guardian's hold and snapshots.
#[derive(Debug, Default)]
struct MazeEpisodeObservation {
    return_tile: Option<WorldTile>,
    reward_baseline: Option<i32>,
    entry: Option<WorldTile>,
    last_maze_tile: Option<WorldTile>,
    tile_changes: u32,
    max_progress: i32,
    progress_seen: bool,
    shrine_seen: bool,
    returned: bool,
    reward_seen: bool,
    released: bool,
}

impl MazeEpisodeObservation {
    fn summary(&self) -> String {
        format!(
            "maze completion episode [return={:?},entry={:?},progress={},changes={},max={},shrine={},returned={},reward={},released={}]",
            self.return_tile,
            self.entry,
            self.progress_seen,
            self.tile_changes,
            self.max_progress,
            self.shrine_seen,
            self.returned,
            self.reward_seen,
            self.released,
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum StallCombatObservation {
    FirstSession,
    FirstBank,
    Returning,
    SecondSession { guard_ready: bool },
}

/// Pre-style trip evidence for [`StepKind::ObserveStyleAcrossLootTrips`].
#[derive(Debug, Clone, Copy, Default)]
struct LootTripObservation {
    /// Most `deposit_id` seen in a fresh open bank during this step. The
    /// cells that use the step start from an empty bank.
    deposited: i32,
    /// Completed trips: observed rises of `deposited`.
    trips: u32,
    /// Successful steals: observed rises of the progress stat's XP.
    steals: u32,
    /// Attack, Strength and Defence XP at the previous frame.
    melee_xp: Option<[i32; 3]>,
    /// The progress stat's XP at the previous frame.
    progress_xp: Option<i32>,
}

/// Bounded continuation for [`StepKind::ObserveCombatAcrossBankReturns`].
#[derive(Debug, Clone, Copy, Default)]
struct CombatBankReturnObservation {
    bank_open_seen: bool,
    bank_generation: u64,
    restarts: u8,
}

const PROTECT_MAGIC_VARP: i32 = 95;
const ZAMORAK_WIZARD: usize = 1007;

#[derive(Debug, Clone)]
struct ProtectWindowObservation {
    seeded: i32,
    ack_tick: Option<u32>,
    kill_tick: Option<u32>,
    kill_evidence: Option<&'static str>,
    prayer_off_tick: Option<u32>,
    in_flight_hits: u32,
    in_flight_max: i32,
    live_hit: [bool; 4],
    violation: Option<&'static str>,
}

impl ProtectWindowObservation {
    fn new(seeded: i32) -> Self {
        Self {
            seeded,
            ack_tick: None,
            kill_tick: None,
            kill_evidence: None,
            prayer_off_tick: None,
            in_flight_hits: 0,
            in_flight_max: 0,
            live_hit: [false; 4],
            violation: None,
        }
    }

    fn magic_on(snap: &GameSnapshot) -> bool {
        snap.varps()
            .iter()
            .find(|varp| varp.index == PROTECT_MAGIC_VARP)
            .is_some_and(|varp| varp.value == 1)
    }

    fn observe(&mut self, snap: &GameSnapshot) {
        if self.violation.is_some() || !snap.ingame() || snap.scene_state() != 2 {
            return;
        }
        let tick = snap.tick();
        if tick == 0 {
            return;
        }
        let magic_on = Self::magic_on(snap);
        if magic_on && self.ack_tick.is_none() {
            self.ack_tick = Some(tick);
        }
        if let Some(ack) = self.ack_tick {
            if !magic_on && self.kill_tick.is_none() {
                self.violation = Some("protect from magic dropped before the kill");
                return;
            }
            if let Some(marks) = snap.hitmarks() {
                for (index, mark) in marks.marks.iter().enumerate() {
                    let is_live = mark.kind == 1 && mark.value > 0 && mark.cycle > marks.loop_cycle;
                    let onset = is_live && !self.live_hit[index];
                    self.live_hit[index] = is_live;
                    if !onset {
                        continue;
                    }
                    if tick > ack && tick <= ack + 2 {
                        self.in_flight_hits = self.in_flight_hits.saturating_add(1);
                        self.in_flight_max = self.in_flight_max.max(mark.value);
                        if self.in_flight_hits > 1 || mark.value > 20 {
                            self.violation = Some("in-flight hit exceeded the 2-tick bound");
                            return;
                        }
                    } else if tick > ack + 2 && magic_on {
                        self.violation = Some("hitsplat after the protected window");
                        return;
                    }
                }
            }
        }
        self.observe_kill(snap, tick);
        if self.kill_tick.is_some() && !magic_on && self.prayer_off_tick.is_none() {
            self.prayer_off_tick = Some(tick);
        }
        if let Some(kill) = self.kill_tick {
            if let Some(off) = self.prayer_off_tick {
                if off > kill + 3 {
                    self.violation = Some("prayers off more than three ticks after the kill");
                }
            } else if tick > kill + 3 {
                self.violation = Some("prayers still on three ticks after the kill");
            }
        }
    }

    fn observe_kill(&mut self, snap: &GameSnapshot, tick: u32) {
        if self.kill_tick.is_some() {
            return;
        }
        if snap.npcs().iter().any(posted_wizard_corpse) {
            self.kill_tick = Some(tick);
            self.kill_evidence = Some("posted_corpse");
        }
    }

    fn ready(&self, snap: &GameSnapshot, names: Option<&ObjNames>) -> bool {
        self.violation.is_none()
            && self.ack_tick.is_some()
            && self.kill_tick.is_some()
            && self
                .prayer_off_tick
                .is_some_and(|off| self.kill_tick.is_some_and(|kill| off <= kill + 3))
            && self.in_flight_hits <= 1
            && Proof::ClueReplaced {
                seeded: self.seeded,
            }
            .check(snap, names)
    }

    fn evidence(&self) -> crate::evidence::ProtectWindowEvidence {
        crate::evidence::ProtectWindowEvidence {
            ack_tick: self.ack_tick,
            in_flight_hits: self.in_flight_hits,
            in_flight_max: self.in_flight_max,
            kill_tick: self.kill_tick,
            kill_evidence: self.kill_evidence,
            prayer_off_tick: self.prayer_off_tick,
        }
    }
}

fn posted_wizard_corpse(npc: &api::snapshot::NpcView) -> bool {
    npc.r#type == Some(ZAMORAK_WIZARD) && npc.total_health > 0 && npc.health == 0
}

fn protect_window_seed(scenario: &Scenario) -> Option<i32> {
    match scenario.proof {
        Proof::ClueProtectWindow { seeded } => Some(seeded),
        _ => scenario.steps.iter().find_map(|step| match step.wait.arm {
            Proof::ClueProtectWindow { seeded } => Some(seeded),
            _ => None,
        }),
    }
}

const LAMP_AWARD_MARKER: &str = "Your wish has been granted!";

#[cfg(test)]
mod protect_window_oracle {
    use super::*;
    use api::snapshot::{NpcView, VarpView, WorldTile};

    fn wizard(health: i32, total_health: i32) -> NpcView {
        NpcView {
            index: 7,
            r#type: Some(ZAMORAK_WIZARD),
            name: Some("Zamorak Wizard".into()),
            actions: vec![Some("Attack".into())],
            tile: WorldTile {
                x: 2947,
                z: 3819,
                level: 0,
            },
            distance: 1,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health,
            total_health,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: true,
            level: 65,
            size: 1,
            network: WorldTile {
                x: 2947,
                z: 3819,
                level: 0,
            },
            x: 0,
            z: 0,
            yaw: 0,
        }
    }

    fn frame(tick: u32, magic: i32, npcs: Vec<NpcView>) -> GameSnapshot {
        let mut snap = GameSnapshot::new();
        snap.seed_ingame(2);
        snap.seed_tick(tick);
        snap.seed_varps(vec![VarpView {
            index: PROTECT_MAGIC_VARP,
            value: magic,
        }]);
        snap.seed_npcs(npcs);
        snap
    }

    #[test]
    fn disappearance_without_a_corpse_is_not_a_kill() {
        let mut window = ProtectWindowObservation::new(2735);
        window.observe(&frame(48, 1, vec![wizard(80, 80)]));
        window.observe(&frame(151, 1, Vec::new()));
        window.observe(&frame(151, 0, Vec::new()));
        assert_eq!(window.kill_tick, None);
        assert_eq!(window.kill_evidence, None);
        assert!(
            !window.ready(&frame(151, 0, Vec::new()), None),
            "a disappearance must not satisfy clue_protect_window"
        );
    }

    #[test]
    fn posted_corpse_latches_the_kill_tick() {
        let mut window = ProtectWindowObservation::new(2735);
        window.observe(&frame(48, 1, vec![wizard(80, 80)]));
        window.observe(&frame(151, 1, vec![wizard(0, 80)]));
        assert_eq!(window.kill_tick, Some(151));
        assert_eq!(window.kill_evidence, Some("posted_corpse"));
    }
}

/// The machine both runners drive. One instance per scenario run.
pub struct ScenarioRunner {
    scenario: Scenario,
    snapshot: GameSnapshot,
    obj_names: Option<Arc<ObjNames>>,
    /// The whole-world router surface (collision + transport graph),
    /// loaded from the baked nav pack; both `Walk` and `Follow` steps
    /// route on it.
    nav_world: Option<Arc<NavWorld>>,
    traveller: Traveller,
    /// WORLD membership for routing. Default false (unknown).
    map_members: bool,
    /// The armed nav route (re-passed each client cycle; the traveller
    /// consumes it when a run starts).
    route: Option<Route>,
    /// Whole-window shot label fired at the terminal state (PASS and
    /// FAIL); `None` for scenarios that only fire `Shot` steps.
    terminal_shot: Option<&'static str>,
    phase: Phase,
    step: usize,
    step_sent: bool,
    /// Observed game tick (`snapshot.tick()`, one per `PLAYER_INFO`) at
    /// which the current `Repeat` step last sent. `None` until the step
    /// sends; reset at every step boundary so a new `Repeat` step can
    /// send at once. Client cycles and dirty snapshots never update it.
    repeat_sent_game_tick: Option<u32>,
    /// Last `snapshot.tick()` this runner observed. The difference on
    /// the next poll is the number of observed game ticks (one per
    /// `PLAYER_INFO`) to credit to the step budget and run total.
    /// Client cycles and dirty snapshots do not advance it.
    last_observed_game_tick: u32,
    /// Observed game ticks waited on the current step's arm (one per
    /// `PLAYER_INFO`). Client cycles and dirty snapshots do not count.
    game_ticks_waited: u32,
    /// Observed game ticks across the whole run (the evidence `ticks`
    /// field). Client cycles and dirty snapshots do not count.
    total_game_ticks: u32,
    started: Instant,
    deadline: Duration,
    /// True when the whole-run deadline ended the run (see
    /// [`ScenarioRunner::deadline_exceeded`]).
    deadline_exceeded: bool,
    /// Wall-clock settle before a step can start: the scene must have
    /// held `scene_state == 2` for this long. A cheat tele's rebuild
    /// drops the scene to 1, and sending the next step before it
    /// re-settles gets `Refused SceneUnavailable` (the 377 `waitSceneReady`
    /// + settle idea).
    scene_settle: Duration,
    /// The wall-clock instant the scene first read `scene_state == 2`,
    /// None while it is not 2. Any drop below 2 resets it.
    scene2_since: Option<Instant>,
    /// The seed waits for a mainland hop to return to its requested landing.
    seed_ready_observed: bool,
    seed_arrival_observed: bool,
    /// Mainland seeds require a completed hop; tests on fixture grids can
    /// explicitly relax this.
    require_mainland_base: bool,
    /// Per-run minted usernames replacing `seed.profiles` (live boots mint
    /// unique names so the engine auto-registers a fresh account instead
    /// of logging the shared `test`). Empty = the registry's seed names.
    live_names: Vec<String>,
    /// Relog: logout IF_BUTTON has gone out once this step. Do not re-press
    /// (that re-arms the 250-frame logout timer and never leaves the game).
    relog_logout_sent: bool,
    /// Relog may finish only after logout was observed or session identity
    /// changed; the old session can already satisfy the step's tab proof.
    relog_session: Option<u64>,
    relog_left_session: bool,
    engine_speed_ms: Option<u32>,
    /// Endpoint reported when the initial world-speed command is sent.
    tick_speed_endpoint: Option<(String, u16)>,
    engine_speed_sent: bool,
    speed_chat_baseline: Option<i32>,
    evidence: Option<Evidence>,
    ground_guard: Option<GroundGuardEvidence>,
    seed_deposits: Vec<SeedDepositEvidence>,
    /// Cumulative skill XP baselines: captured at StartScript for its later
    /// watches, otherwise when the first XP watch for that skill begins.
    xp_baselines: Vec<(i32, i32)>,
    /// Baseline for the current [`Proof::FreshStatXpGain`] step only. Cleared
    /// at every step and session boundary so prior work cannot satisfy it.
    fresh_xp_baseline: Option<(i32, i32)>,
    /// Chat-sequence baseline for the current FreshTutorial, FreshChat,
    /// FreshChatSincePreviousStep, or FreshQuestCompletion wait. Latched
    /// when the step begins (the previous step for
    /// FreshChatSincePreviousStep) and cleared at every step and session
    /// boundary, so a stale line cannot satisfy it.
    fresh_chat_baseline: Option<i32>,
    /// Chat-sequence baseline for the *next* step when its arm is
    /// [`Proof::FreshChatSincePreviousStep`]. Latched when the current
    /// step begins and handed to [`Self::fresh_chat_baseline`] unchanged
    /// when the run advances into that step, so a line sent while the
    /// previous watch ran stays in scope but older history does not.
    next_fresh_chat_baseline: Option<i32>,
    /// Latched per step for [`Proof::VisitedWithin`]: the first snapshot
    /// that shows the player inside the proof rectangle sets it. Cleared
    /// at every step and session boundary, like the chat baseline above.
    visited_within: bool,
    /// Native host-hold + snapshot facts for the current lamp witness step.
    /// This is deliberately not part of [`GameSnapshot`].
    lamp_episode: Option<LampEpisodeObservation>,
    /// Native host-hold + snapshot ordering for the current Maze witness.
    maze_episode: Option<MazeEpisodeObservation>,
    /// At most one catch-less stall session may be followed by a bank retry.
    stall_combat: Option<StallCombatObservation>,
    /// Pre-style Loot-count trips for the current style watch.
    loot_trips: Option<LootTripObservation>,
    /// Fresh combat windows anchored on completed bank returns.
    combat_bank_returns: Option<CombatBankReturnObservation>,
    /// Random-event Guardian episode, armed by an
    /// [`StepKind::ObserveRandomEvent`] step and kept for the terminal receipt.
    random_event: Option<RandomEventEpisode>,
    /// Host-fed Sherlock protect-window oracle, armed when the scenario names it.
    protect_window: Option<ProtectWindowObservation>,
    /// Newest chat sequence at StartScript when the scenario sets
    /// `fail_on_death`: a later death message fails the run at once.
    death_chat_baseline: Option<i32>,
    /// StartScript has begun: paint published before it cannot belong to
    /// this run's card.
    script_started: bool,
    /// Current Stop-step host observation, never inferred from game stats.
    script_idle: bool,
    /// The prefixes the scenario's [`Proof::ScriptReceipt`] watches name,
    /// collected once at construction (empty, unallocated, for every
    /// scenario without one), so the client-cycle pump never walks the steps.
    receipt_prefixes: Vec<&'static str>,
    /// File-card receipt rows those watches latched from the driven slot's
    /// published paint (first row per prefix). Host-fed through
    /// [`ScenarioRunner::observe_script_paint`]; not snapshot state.
    script_receipts: Vec<String>,
    /// Compiled Start reached Running. Host-fed through
    /// [`ScenarioRunner::observe_script_running`]; [`Proof::ScriptRunning`]
    /// reads this latch, never the snapshot.
    script_running: bool,
    /// Whole-window shot sink: fired once when a `StepKind::Shot` step's
    /// arm holds, with the label and the terminal snapshot. The headed
    /// panel fills this with its window capture; the headless twin keeps
    /// the default no-op.
    #[allow(clippy::type_complexity)]
    shot_sink: Option<Box<dyn FnMut(&str, &GameSnapshot) + Send>>,
    terminal_cleanup: Option<TerminalCleanup>,
}

impl ScenarioRunner {
    /// A runner for `scenario`; the whole-world router surface (collision
    /// and transport graph) loads from the standard pack path (`None` when
    /// no pack, so `Walk`/`Follow` steps fail with a clear message).
    pub fn new(scenario: Scenario) -> Self {
        let pack = crate::default_pack_path();
        let nav_world = NavWorld::load_pack(&pack).ok().map(Arc::new);
        Self::with_data(scenario, nav_world)
    }

    /// Share the immutable navigation world without cloning its contents.
    pub fn shared_world(&self) -> Option<Arc<NavWorld>> {
        self.nav_world.clone()
    }

    /// Runner with injected nav data: the whole-world [`NavWorld`]
    /// (collision + transport graph) both `Walk` and `Follow` steps route
    /// on. `None` keeps no world loaded, so a nav step fails with a clear
    /// "no nav world" message (the live [`ScenarioRunner::new`] loads the
    /// default pack path).
    pub fn with_world(scenario: Scenario, nav_world: Option<Arc<NavWorld>>) -> Self {
        Self::with_data(scenario, nav_world)
    }

    /// Bind WORLD membership for routing. Unknown/false is the default.
    pub fn set_map_members(&mut self, map_members: bool) {
        self.map_members = map_members;
    }

    /// Record the endpoint for diagnostics when the world-wide speed command fires.
    /// This is logging metadata only; it does not restrict command dispatch.
    pub fn set_tick_speed_endpoint(&mut self, host: &str, port: u16) {
        self.tick_speed_endpoint = Some((host.to_owned(), port));
    }

    fn with_data(scenario: Scenario, nav_world: Option<Arc<NavWorld>>) -> Self {
        // Settings fields are Copy, so read them out before `scenario`
        // moves into the runner below.
        let deadline = scenario.settings.deadline;
        let terminal_shot = scenario.settings.terminal_shot;
        let require_mainland_base =
            scenario.seed.mainland || scenario.settings.require_mainland_base;
        let engine_speed_ms = scenario.settings.nav.engine_speed_ms;
        let receipt_prefixes = receipt_prefixes(&scenario);
        Self {
            scenario,
            snapshot: GameSnapshot::new(),
            obj_names: None,
            nav_world,
            traveller: Traveller::new(),
            map_members: false,
            route: None,
            terminal_shot,
            phase: Phase::Seeding,
            step: 0,
            step_sent: false,
            repeat_sent_game_tick: None,
            last_observed_game_tick: 0,
            game_ticks_waited: 0,
            total_game_ticks: 0,
            started: Instant::now(),
            deadline,
            deadline_exceeded: false,
            scene_settle: Duration::from_secs(2),
            scene2_since: None,
            seed_ready_observed: false,
            seed_arrival_observed: false,
            require_mainland_base,
            live_names: Vec::new(),
            relog_logout_sent: false,
            relog_session: None,
            relog_left_session: false,
            engine_speed_ms,
            tick_speed_endpoint: None,
            speed_chat_baseline: None,
            engine_speed_sent: false,
            evidence: None,
            ground_guard: None,
            seed_deposits: Vec::new(),
            xp_baselines: Vec::new(),
            fresh_xp_baseline: None,
            fresh_chat_baseline: None,
            next_fresh_chat_baseline: None,
            visited_within: false,
            lamp_episode: None,
            maze_episode: None,
            stall_combat: None,
            loot_trips: None,
            combat_bank_returns: None,
            random_event: None,
            protect_window: None,
            death_chat_baseline: None,
            script_started: false,
            receipt_prefixes,
            script_receipts: Vec::new(),
            script_running: false,
            script_idle: false,
            shot_sink: None,
            terminal_cleanup: None,
        }
    }

    /// Install the whole-window shot sink (headed: the panel's window
    /// capture bridge; headless: a no-op). Fired when a `Shot` step's
    /// arm holds.
    #[allow(clippy::type_complexity)]
    pub fn set_shot_sink(&mut self, sink: Box<dyn FnMut(&str, &GameSnapshot) + Send>) {
        self.shot_sink = Some(sink);
    }

    /// Fire the shot sink at the terminal state (PASS **and** FAIL) with
    /// `label`, in addition to any `Shot` steps. The headed panel arms
    /// this for scenarios that need a terminal capture; the headless twin
    /// keeps the default (no-op) sink.
    pub fn set_terminal_shot(&mut self, label: &'static str) {
        self.terminal_shot = Some(label);
    }

    /// The armed terminal-shot label, `None` when only `Shot` steps fire.
    /// The panel reads it to hold a FAIL exit until the capture drains.
    pub fn terminal_shot(&self) -> Option<&'static str> {
        self.terminal_shot
    }

    /// The scenario under test (for the headed recording sidecar).
    pub fn scenario_name(&self) -> &'static str {
        self.scenario.name
    }

    /// Recording sidecar position: `(step_index, total_steps, step_name)`.
    /// Pre-start reads `seeding`, the proof tail reads `proving`, terminal
    /// speed restoration reads `cleaning`, and a finished run reads `done`.
    /// Every video timestamp lines up with the step trace even outside run steps.
    pub fn record_step(&self) -> (usize, usize, &'static str) {
        let total = self.scenario.steps.len();
        match self.phase {
            Phase::Seeding => (0, total, "seeding"),
            Phase::Running => self.scenario.steps.get(self.step).map_or_else(
                || (self.step.min(total), total, "running"),
                |step| (self.step, total, step.name),
            ),
            Phase::Proving => (total, total, "proving"),
            Phase::Cleaning => (total, total, "cleaning"),
            Phase::Done => (total, total, "done"),
        }
    }

    /// Observed game ticks across the whole run (the evidence `ticks`
    /// field): one per `PLAYER_INFO`, counted while the run steps run.
    /// Client cycles (frontend polls) and dirty snapshots do not count.
    /// The recording sidecar lines each video timestamp up with this count.
    pub fn total_ticks(&self) -> u32 {
        self.total_game_ticks
    }

    /// Existing isolate self-stop reason the headed/TUI live path waits for
    /// after game-state proofs, when the scenario asked for a clean stop.
    pub fn wait_script_stop(&self) -> Option<&'static str> {
        self.scenario.settings.wait_script_stop
    }

    /// The shared obj-id → name table for `Item` predicates and the
    /// evidence's inventory names.
    pub fn set_obj_names(&mut self, names: Arc<ObjNames>) {
        self.obj_names = Some(names);
    }

    /// Override the whole-run wall-clock deadline (tests use short ones).
    pub fn set_deadline(&mut self, deadline: Duration) {
        self.deadline = deadline;
    }

    /// The whole-run wall-clock deadline (from `scenario.settings` unless
    /// overridden by [`ScenarioRunner::set_deadline`]).
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// True when the whole-run deadline ended the run. A step, proof, or
    /// other failure leaves it false, even if a caller reads the status
    /// after the deadline. The `Failed` text alone does not tell the
    /// causes apart.
    pub fn deadline_exceeded(&self) -> bool {
        self.deadline_exceeded
    }

    /// Per-run minted usernames replacing `seed.profiles` (live boots mint
    /// unique names so the engine auto-registers a fresh account instead
    /// of logging the shared `test`). Empty = the registry's seed names.
    pub fn set_live_names(&mut self, names: &[String]) {
        debug_assert_eq!(
            names.len(),
            self.scenario.seed.profiles.len(),
            "live names must cover every seed profile"
        );
        self.live_names = names.to_vec();
    }

    /// The login name of seed slot `index` (minted live name when armed,
    /// else the registry's seed name).
    fn seed_name(&self, index: usize) -> &str {
        self.live_names
            .get(index)
            .map(String::as_str)
            .unwrap_or(self.scenario.seed.profiles[index].0)
    }

    /// Override the scene-settle wall-clock hold. Tests use
    /// `Duration::ZERO` so the gate reduces to `scene_state == 2`.
    pub fn set_scene_settle(&mut self, settle: Duration) {
        self.scene_settle = settle;
    }

    /// Relax the mainland build-base seed gate (>= 3000). Tests on
    /// fixture grids call this; live runners keep the gate so a walk is
    /// never armed from the tutorial island.
    pub fn no_mainland_gate(&mut self) {
        self.require_mainland_base = false;
    }

    /// The profile the runner drives (`seed.profiles[0]`, or the minted
    /// live name when armed); client-cycle hooks observe only this slot.
    pub fn profile_name(&self) -> &str {
        self.seed_name(0)
    }

    /// Names of every slot owned by this run, including companion seeds.
    /// Frontends use this to inspect producer status without matching
    /// unrelated user slots.
    pub fn owned_profile_names(&self) -> Vec<String> {
        let mut names = vec![self.profile_name().to_string()];
        names.extend(
            self.scenario
                .companions
                .iter()
                .map(|companion| self.seed_name(companion.profile).to_string()),
        );
        names
    }

    /// Whether `name` is the slot this runner drives.
    pub fn drives(&self, name: &str) -> bool {
        name == self.profile_name()
    }

    /// The Follow/Walk route currently armed, if any. The headed panel
    /// paints this as the red baked path; WalkTo's arm is a different
    /// traveller and does not see live-scenario hops.
    pub fn armed_route(&self) -> Option<&Route> {
        self.route.as_ref()
    }

    /// The current walk (or transport-approach) aim, when a hop is out.
    pub fn current_aim(&self) -> Option<WorldTile> {
        self.traveller.current_aim()
    }

    /// The companion slot at `index`'s profile name
    /// (`seed.profiles[companion.profile].0`).
    pub fn companion_profile_name(&self, index: usize) -> &str {
        let profile = self.scenario.companions[index].profile;
        self.seed_name(profile)
    }

    /// The companion slot whose profile name is `name`, or `None` when no
    /// companion has it. The driven slot (profile 0) is never a companion.
    /// Client-cycle hooks dispatch to [`ScenarioRunner::tick`] for the
    /// driven slot, or [`ScenarioRunner::companion_tick`] for a companion.
    pub fn companion_for(&self, name: &str) -> Option<usize> {
        self.scenario
            .companions
            .iter()
            .position(|c| self.seed_name(c.profile) == name)
    }

    /// Run companion `index`'s hook once per client cycle on `client`. The
    /// caller delivers one frame per call, exactly like [`tick`].
    pub fn companion_tick(&mut self, index: usize, client: &mut Client) {
        (self.scenario.companions[index].per_frame)(client);
    }

    /// True while the runner is on [`StepKind::StartScript`]: panel/TUI
    /// live pumps call `script_start_load` once, then the step wait.
    pub fn on_start_script(&self) -> bool {
        matches!(self.phase, Phase::Running)
            && self.step < self.scenario.steps.len()
            && matches!(self.current_step().kind, StepKind::StartScript)
    }

    /// True while the live pump must revoke the current script before the
    /// next client-cycle observation.
    pub fn on_stop_script(&self) -> bool {
        matches!(self.phase, Phase::Running)
            && self.step < self.scenario.steps.len()
            && matches!(self.current_step().kind, StepKind::StopScript)
    }

    /// Observe Stop's actual script-slot result before the next
    /// client-cycle observation.
    pub fn observe_script_idle(&mut self, idle: bool) {
        if self.on_stop_script() {
            self.script_idle = idle;
        }
    }

    /// Whether the live pump should hand this runner the driven slot's
    /// published paint: the card has started and a
    /// [`Proof::ScriptReceipt`] watch has not latched its row yet. False
    /// for every scenario without a receipt watch, so ordinary runs never
    /// read paint.
    pub fn wants_script_paint(&self) -> bool {
        self.script_started
            && !matches!(self.phase, Phase::Done)
            && self.receipt_prefixes.len() > self.script_receipts.len()
    }

    /// Latch that the compiled Start reached Running. The live pump calls
    /// this when `poll_start` settles Ready; [`Proof::ScriptRunning`] then
    /// holds.
    pub fn observe_script_running(&mut self) {
        self.script_running = true;
    }

    /// Fail the current step with `reason` (compiled Start Rejected/Failed).
    /// No-op after the run has already finished.
    pub fn fail_start(&mut self, reason: &str) {
        if matches!(self.phase, Phase::Done) {
            return;
        }
        if matches!(self.phase, Phase::Running) && self.step < self.scenario.steps.len() {
            self.finish_fail(&format!(
                "step {} ({}): {reason}",
                self.step + 1,
                self.current_step().name
            ));
        } else {
            self.finish_fail(reason);
        }
    }

    /// Latch the first published paint row that starts with each receipt
    /// prefix the scenario watches. The live pump calls this with the
    /// driven slot's current paint frame before the next client-cycle
    /// observation. Paint published before StartScript is ignored.
    pub fn observe_script_paint<'a>(&mut self, lines: impl IntoIterator<Item = &'a str>) {
        if !self.script_started || matches!(self.phase, Phase::Done) {
            return;
        }
        for line in lines {
            let Some(prefix) = self
                .receipt_prefixes
                .iter()
                .find(|prefix| line.starts_with(**prefix))
            else {
                continue;
            };
            if !self
                .script_receipts
                .iter()
                .any(|row| row.starts_with(prefix))
            {
                self.script_receipts.push(line.to_string());
            }
        }
    }

    /// The terminal evidence record, `None` until PASS/FAIL.
    pub fn evidence(&self) -> Option<&Evidence> {
        self.evidence.as_ref()
    }

    pub fn status(&self) -> RunnerStatus {
        match self.phase {
            Phase::Done => match self.evidence.as_ref().map(|e| e.outcome) {
                Some("FAIL" | "UNAVAILABLE") => RunnerStatus::Failed(
                    self.evidence
                        .as_ref()
                        .and_then(|e| e.message.clone())
                        .unwrap_or_default(),
                ),
                _ => RunnerStatus::Passed,
            },
            Phase::Seeding => RunnerStatus::Seeding,
            Phase::Running => RunnerStatus::Running {
                step: self.step,
                total: self.scenario.steps.len(),
            },
            Phase::Proving | Phase::Cleaning => RunnerStatus::Running {
                step: self.scenario.steps.len(),
                total: self.scenario.steps.len(),
            },
        }
    }

    /// One client-cycle observation of `client`: rebuild the snapshot, send
    /// the current step's action if it has not gone out yet, hop the traveller,
    /// and check the step arm / proof predicate. Never sleeps. A seed-completion
    /// client cycle falls through into the run logic, so the send goes out in
    /// the same cycle the seed completes.
    pub fn tick(&mut self, client: &mut Client) {
        self.tick_with_hold(client, false);
    }

    /// Like [`ScenarioRunner::tick`], one client-cycle pump, but skip
    /// `Traveller::follow` while `hold` is set (guardian hold — the armed route stays latched).
    pub fn tick_with_hold(&mut self, client: &mut Client, hold: bool) {
        self.tick_inner(client, hold);
        if matches!(self.phase, Phase::Done) {
            // A follow failure may still be consumed by this poll's remaining
            // checks. Release only after all existing callbacks and evidence.
            self.snapshot = GameSnapshot::new();
        }
    }

    fn tick_inner(&mut self, client: &mut Client, hold: bool) {
        if matches!(self.phase, Phase::Done) {
            return;
        }
        let dirty = self.snapshot.rebuild(client);
        // Observed game ticks since the last poll: `snapshot.tick()`
        // advances once per `PLAYER_INFO` (the server's world cycle),
        // never on a client cycle or a dirty snapshot alone. Step
        // budgets and the run total credit this delta in the run block
        // below; polls that return before it (cleaning, scene settle)
        // do not count toward a step budget.
        let observed_game_tick = self.snapshot.tick();
        let game_tick_delta = observed_game_tick.wrapping_sub(self.last_observed_game_tick);
        self.last_observed_game_tick = observed_game_tick;
        if matches!(self.phase, Phase::Cleaning) {
            self.tick_terminal_cleanup(client);
            return;
        }
        if !self.snapshot.ingame() {
            self.fresh_xp_baseline = None;
            self.fresh_chat_baseline = None;
            self.next_fresh_chat_baseline = None;
            self.visited_within = false;
            self.lamp_episode = None;
            self.maze_episode = None;
        }
        self.retry_xp_baselines();
        self.retry_chat_baseline();
        // Scene-settle tracking: the wall-clock instant the scene first
        // held `scene_state == 2`; any drop below 2 (a tele's rebuild)
        // resets it.
        if self.snapshot.scene_state() == 2 {
            if self.scene2_since.is_none() {
                self.scene2_since = Some(Instant::now());
            }
        } else {
            self.scene2_since = None;
        }
        if matches!(self.phase, Phase::Seeding) && self.require_mainland_base {
            let was_ready = self.seed_ready_observed;
            if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
                self.seed_ready_observed = true;
            }
            // Do not accept an unrelated scene rebuild as the hop. The
            // mainlandAccount tele lands at this exact tile; checking it on
            // a later observation also handles a hop whose rebuild is too
            // fast to expose an intermediate scene_state.
            if was_ready && snapshot_tile(&self.snapshot) == Some(crate::MAINLAND_LANDING) {
                self.seed_arrival_observed = true;
            }
        }
        if matches!(self.phase, Phase::Seeding) && self.seed_done() {
            self.phase = Phase::Running;
            self.begin_step();
        }
        if matches!(self.phase, Phase::Running) {
            // Scene-settle gate: never start a step until the scene has
            // held `scene_state == 2` for `scene_settle`. A cheat tele's
            // rebuild drops the scene to 1 and a send before it re-settles
            // gets `Refused SceneUnavailable`. The gate applies to every
            // step, so a tele's `Arrived` arm also waits for the scene to
            // settle before advancing. No send, no budget counting, no
            // step advance.
            let settled = match self.scene2_since {
                Some(since) => since.elapsed() >= self.scene_settle,
                None => false,
            };
            // Relog is off-world between logout and login; do not hold the
            // scene-2 settle gate or the login never goes out.
            let relog_off_world =
                matches!(self.current_step().kind, StepKind::Relog) && !self.snapshot.ingame();
            if !settled && !relog_off_world {
                return;
            }
            if !self.engine_speed_sent {
                if let Some(ms) = self.engine_speed_ms {
                    // Record the pre-send chat mark before the command: the
                    // confirm step accepts only a confirmation newer than
                    // it, so a matching line already in the ring is stale.
                    self.speed_chat_baseline =
                        Some(crate::quest_fast::chat_baseline(&self.snapshot));
                    if let Err(error) = crate::quest_fast::send_tick_speed(client, ms) {
                        self.finish_fail(&error);
                        return;
                    }
                    self.log_tick_speed_endpoint(ms);
                }
                self.engine_speed_sent = true;
            }
            self.fire_poll_sustains(client);
            // Guardian hold freezes follow only; sends/arms still run so a
            // route armed under hold resumes when it lifts.
            if dirty && !hold {
                self.step_route(client);
            }
            // A `Repeat` step (and the native seed deposit, which
            // re-attempts the same way) sends at most once per
            // observed game tick (one `PLAYER_INFO`). Frontend polls
            // run at the client-cycle rate, so an unguarded repeat
            // floods the engine's per-tick input budget and delays
            // the next step's command behind the repeats. The first
            // send of a step goes out at once
            // (`repeat_sent_game_tick` is `None`).
            let repeat = matches!(
                self.current_step().kind,
                StepKind::Repeat { .. } | StepKind::NativeSeedDeposit { .. }
            );
            let repeat_ready = !repeat || self.repeat_sent_game_tick != Some(self.snapshot.tick());
            if !self.step_sent && repeat_ready {
                self.fire_nav_step_sustains(client);
                if let Err(msg) = self.send_current(client) {
                    self.finish_fail(&format!(
                        "step {} ({}): {msg}",
                        self.step + 1,
                        self.current_step().name
                    ));
                    return;
                }
                if repeat {
                    self.repeat_sent_game_tick = Some(self.snapshot.tick());
                }
                // A `DrainDialogs` janitor re-sends every client cycle: it
                // answers whatever `p_choice` dialog is up (the quest-seed
                // prompts) and closes the quest-scroll modals until its
                // arm holds, so the send stays armed — and the step's
                // game-tick budget keeps counting.
                self.step_sent |= !matches!(
                    self.current_step().kind,
                    StepKind::DrainDialogs { .. }
                        | StepKind::Relog
                        | StepKind::Repeat { .. }
                        | StepKind::NativeSeedDeposit { .. }
                );
                if self.step_sent {
                    self.game_ticks_waited = 0;
                }
            }
            // Budgets count observed game ticks only. Route polling above
            // and the proof checks below still run on every poll; a burst
            // of dirty snapshots inside one game tick does not use budget.
            self.game_ticks_waited = self.game_ticks_waited.saturating_add(game_tick_delta);
            self.total_game_ticks = self.total_game_ticks.saturating_add(game_tick_delta);
            if dirty {
                if let Some(window) = self.protect_window.as_mut() {
                    window.observe(&self.snapshot);
                }
                self.observe_visited();
            }
            if let Some(message) = self
                .protect_window
                .as_ref()
                .and_then(|window| window.violation)
            {
                self.finish_fail(&format!(
                    "step {} ({}): protect window: {message}",
                    self.step + 1,
                    self.current_step().name
                ));
                return;
            }
            if self.death_chat_baseline.is_some_and(|baseline| {
                crate::proof::fresh_chat_contains(
                    &self.snapshot,
                    baseline,
                    crate::random_episode::DEATH_MESSAGE,
                )
            }) {
                self.finish_fail(&format!(
                    "step {} ({}): the bot died after Start at {:?}",
                    self.step + 1,
                    self.current_step().name,
                    self.snapshot.tile()
                ));
                return;
            }
            let (arm, budget) = {
                let wait = &self.current_step().wait;
                (wait.arm, wait.budget_ticks)
            };
            self.observe_ground_guard(arm);
            if let Err(message) = self.observe_loot_trips(arm) {
                self.finish_fail(&format!(
                    "step {} ({}): {message}",
                    self.step + 1,
                    self.current_step().name
                ));
                return;
            }
            let bank_return_ready = match self.observe_combat_bank_returns(arm) {
                Ok(ready) => ready,
                Err(message) => {
                    self.finish_fail(&format!(
                        "step {} ({}): {message}",
                        self.step + 1,
                        self.current_step().name
                    ));
                    return;
                }
            };
            let waited_game_ticks = if matches!(arm, Proof::GroundItemIdsAbsentNear { .. }) {
                self.ground_guard
                    .as_ref()
                    .map_or(0, |guard| guard.ticks_waited)
            } else {
                self.game_ticks_waited
            };
            if matches!(self.current_step().kind, StepKind::Relog) {
                self.relog_left_session |= !self.snapshot.ingame()
                    || self
                        .relog_session
                        .is_some_and(|session| client.gens.session != session);
            }
            let arm_holds = self.holds(arm);
            let stall_combat_holds = match self.observe_stall_combat(arm_holds) {
                Ok(holds) => holds,
                Err(message) => {
                    self.finish_fail(message);
                    return;
                }
            };
            let random_event_holds = match self.observe_random_event(client, hold) {
                Verdict::Pending => false,
                Verdict::Pass => true,
                Verdict::Fail(message) => {
                    self.finish_fail(&format!(
                        "step {} ({}): {message}",
                        self.step + 1,
                        self.current_step().name
                    ));
                    return;
                }
                Verdict::Unavailable(message) => {
                    self.finish_unavailable(&message);
                    return;
                }
            };
            let native_episode_holds = random_event_holds
                && self.observe_lamp_redemption(hold).unwrap_or(true)
                && self.observe_maze_completion(hold).unwrap_or(true)
                && stall_combat_holds
                && bank_return_ready;
            let arm_holds = match &self.current_step().kind {
                StepKind::Await { ready, .. } => ready(&self.snapshot),
                StepKind::Relog => {
                    self.relog_left_session
                        && arm_holds
                        && (arm == Proof::LoggedOut || self.snapshot.ingame())
                }
                _ => arm_holds,
            };
            if native_episode_holds && arm_holds {
                // A nav step only advances once its follow has
                // terminated: the arm can hold on a snapshot the
                // traveller has not polled yet — the essence-mine entry
                // lands mid-scene-rebuild, the settle gate skips the
                // follow's poll, and the arm alone would advance with the
                // traveller still watching the hop, dropping the session
                // latch (the exit route then fails closed).
                let nav_done = match &self.current_step().kind {
                    StepKind::Walk { .. }
                    | StepKind::Follow { .. }
                    | StepKind::FollowTele { .. } => self.route.is_none(),
                    _ => true,
                };
                if nav_done {
                    if matches!(self.current_step().kind, StepKind::NativeSeedDeposit { .. }) {
                        self.trace_seed_deposit("ProofObserved");
                    }
                    let shot_label = match &self.current_step().kind {
                        StepKind::Shot { label } => Some(*label),
                        _ => None,
                    };
                    if let Some(label) = shot_label {
                        if let Some(sink) = self.shot_sink.as_mut() {
                            sink(label, &self.snapshot);
                        }
                    }
                    self.advance_step();
                }
            } else if waited_game_ticks >= budget {
                let tick_unit = if budget == 1 {
                    "game tick"
                } else {
                    "game ticks"
                };
                if matches!(arm, Proof::GroundItemIdsAbsentNear { .. }) {
                    self.finish_fail(&format!(
                        "fixture interference: step {} ({}): public loot field did not become clean within {budget} observed {tick_unit}",
                        self.step + 1,
                        self.current_step().name,
                    ));
                    return;
                }
                let missing = self
                    .lamp_episode
                    .as_ref()
                    .map(LampEpisodeObservation::summary)
                    .or_else(|| {
                        self.maze_episode
                            .as_ref()
                            .map(MazeEpisodeObservation::summary)
                    })
                    .or_else(|| {
                        self.stall_combat
                            .map(|state| format!("{} [stall_combat={state:?}]", arm.name()))
                    })
                    .or_else(|| {
                        self.loot_trips.map(|trips| {
                            format!(
                                "{} [pre-style trips={} deposited={} steals={}]",
                                arm.name(),
                                trips.trips,
                                trips.deposited,
                                trips.steals
                            )
                        })
                    })
                    .or_else(|| {
                        matches!(
                            self.current_step().kind,
                            StepKind::ObserveRandomEvent { .. }
                        )
                        .then(|| self.random_event.as_ref().map(RandomEventEpisode::summary))
                        .flatten()
                    })
                    .unwrap_or_else(|| arm.name());
                self.finish_fail(&format!(
                    "step {} ({}): {} not seen within {} {}",
                    self.step + 1,
                    self.current_step().name,
                    missing,
                    budget,
                    tick_unit
                ));
            }
        }
        if matches!(self.phase, Phase::Proving) && self.holds(self.scenario.proof) {
            self.finish_pass();
        }
        if !matches!(self.phase, Phase::Done | Phase::Cleaning)
            && self.started.elapsed() > self.deadline
        {
            self.deadline_exceeded = true;
            self.finish_fail(&format!(
                "{}: deadline {:?} exceeded",
                self.scenario.name, self.deadline
            ));
        }
    }

    /// Seed complete: ingame, scene 2, a mainland build base, and the
    /// requested mainland landing. The tutorial island's build base is also
    /// `>= 3000`, so the base heuristic alone cannot release the gate.
    fn seed_done(&self) -> bool {
        if !self.snapshot.ingame() || self.snapshot.scene_state() != 2 {
            return false;
        }
        if !self.require_mainland_base {
            return true;
        }
        if !self.seed_arrival_observed {
            return false;
        }
        let base_ok = self
            .snapshot
            .base()
            .is_some_and(|(bx, bz)| bx >= 3000 && bz >= 3000);
        let on_grid = match (self.nav_world.as_ref(), self.snapshot.tile()) {
            (Some(w), Some((x, z, l))) => {
                let o = w.collision.origin;
                l == o.level
                    && x >= o.x
                    && z >= o.z
                    && x < o.x + w.collision.width as i32
                    && z < o.z + w.collision.height as i32
            }
            // No world loaded: fall back to the base heuristic; a `Walk`
            // step fails with a clear "no nav world" message later.
            (None, _) => true,
            (Some(_), None) => false,
        };
        base_ok && on_grid
    }

    fn current_step(&self) -> &crate::Step {
        &self.scenario.steps[self.step]
    }

    fn begin_step(&mut self) {
        self.step_sent = false;
        self.repeat_sent_game_tick = None;
        self.relog_logout_sent = false;
        self.relog_session = None;
        self.relog_left_session = false;
        self.game_ticks_waited = 0;
        self.traveller.clear();
        self.route = None;
        self.fresh_xp_baseline = None;
        self.fresh_chat_baseline = None;
        if matches!(
            self.current_step().wait.arm,
            Proof::FreshChatSincePreviousStep { .. }
        ) {
            // Hand over the baseline latched when the previous step began;
            // the capture below fills it from this step's own start when
            // there is none (first step, or a session boundary between).
            self.fresh_chat_baseline = self.next_fresh_chat_baseline.take();
        }
        self.visited_within = false;
        self.capture_chat_baseline(self.current_step().wait.arm);
        self.capture_next_chat_baseline();
        if matches!(
            self.current_step().wait.arm,
            Proof::GroundItemIdsAbsentNear { .. }
        ) {
            self.ground_guard = Some(GroundGuardEvidence {
                start_tick: self.snapshot.tick(),
                ticks_waited: 0,
                budget_game_ticks: self.current_step().wait.budget_ticks,
                items_seen: Vec::new(),
            });
        }
        self.lamp_episode = match self.current_step().kind {
            StepKind::ObserveLampRedemption { reward_stat, .. } => Some(LampEpisodeObservation {
                reward_baseline: self.stat_xp(reward_stat),
                clear_before_dialogue: !Proof::ActiveContinue
                    .check(&self.snapshot, self.obj_names.as_deref()),
                ..LampEpisodeObservation::default()
            }),
            _ => None,
        };
        self.maze_episode = match self.current_step().kind {
            StepKind::ObserveMazeCompletion { spawns, .. } => {
                let ready = self.snapshot.ingame() && self.snapshot.scene_state() == 2;
                let return_tile = snapshot_tile(&self.snapshot)
                    .filter(|tile| ready && !on_maze_square(*tile, spawns));
                Some(MazeEpisodeObservation {
                    return_tile,
                    reward_baseline: return_tile.map(|_| inventory_total(&self.snapshot)),
                    ..MazeEpisodeObservation::default()
                })
            }
            _ => None,
        };
        self.stall_combat = matches!(
            self.current_step().kind,
            StepKind::ObserveStallCombat { .. }
        )
        .then_some(StallCombatObservation::FirstSession);
        self.loot_trips = matches!(
            self.current_step().kind,
            StepKind::ObserveStyleAcrossLootTrips { .. }
        )
        .then(LootTripObservation::default);
        self.combat_bank_returns = matches!(
            self.current_step().kind,
            StepKind::ObserveCombatAcrossBankReturns { .. }
        )
        .then_some(CombatBankReturnObservation {
            bank_generation: self.snapshot.bank_session_generation(),
            ..CombatBankReturnObservation::default()
        });
        if let StepKind::ObserveRandomEvent { cell } = self.current_step().kind {
            // The cells run on the 289 engine (`tools/engine-patches/289`);
            // missing content data leaves the evade range unknown, which the
            // oracle reports instead of guessing.
            let data = api::game_data::for_revision(client::io::ClientRevision::R289).ok();
            self.random_event = Some(RandomEventEpisode::new(cell).with_game_data(data));
        }
        if matches!(self.current_step().kind, StepKind::StopScript) {
            self.script_idle = false;
            self.script_started = false;
            self.script_receipts.clear();
        }
        if matches!(self.current_step().kind, StepKind::StartScript) {
            self.script_started = true;
            self.script_running = false;
            self.script_receipts.clear();
            if let Some(seeded) = protect_window_seed(&self.scenario) {
                self.protect_window = Some(ProtectWindowObservation::new(seeded));
            }
            self.death_chat_baseline = self
                .scenario
                .settings
                .fail_on_death
                .then(|| crate::tutorial::chat_baseline(&self.snapshot));
            // Several skills can advance before their sequential watches
            // begin (a catching Guard can die before the first cake is stolen).
            // Seed XP is excluded; fresh return-trip watches stay step-local.
            self.xp_baselines.clear();
            for index in self.step + 1..self.scenario.steps.len() {
                let proof = self.scenario.steps[index].wait.arm;
                if matches!(proof, Proof::StatXpGain { .. }) {
                    self.capture_xp_baseline(proof);
                }
            }
            if matches!(self.scenario.proof, Proof::StatXpGain { .. }) {
                self.capture_xp_baseline(self.scenario.proof);
            }
        }
        self.capture_xp_baseline(self.current_step().wait.arm);
    }

    /// Whether `proof` holds on this tick: snapshot predicates against the
    /// runner's XP baselines, File-card receipts against the host-fed latch.
    fn holds(&self, proof: Proof) -> bool {
        if self.on_stop_script() && !self.script_idle {
            return false;
        }
        match proof {
            Proof::ScriptIdle => self.script_idle,
            Proof::ScriptReceipt { prefix } => self
                .script_receipts
                .iter()
                .any(|row| row.starts_with(prefix)),
            Proof::ScriptRunning => self.script_running,
            Proof::ClueProtectWindow { .. } => self
                .protect_window
                .as_ref()
                .is_some_and(|window| window.ready(&self.snapshot, self.obj_names.as_deref())),
            // Freshness needs the runner's pre-send baseline; a bare
            // snapshot check would accept a stale matching line.
            Proof::WorldSpeedChanged { ms } => self.speed_chat_baseline.is_some_and(|baseline| {
                crate::quest_fast::tick_speed_confirmed(&self.snapshot, baseline, ms)
            }),
            Proof::FreshTutorial => match self.fresh_chat_baseline {
                Some(baseline) => crate::tutorial::tutorial_confirmed(&self.snapshot, baseline),
                None => false,
            },
            Proof::FreshChat { needle } | Proof::FreshChatSincePreviousStep { needle } => {
                match self.fresh_chat_baseline {
                    Some(baseline) => {
                        crate::proof::fresh_chat_contains(&self.snapshot, baseline, needle)
                    }
                    None => false,
                }
            }
            Proof::FreshQuestCompletion {
                chat_needle,
                main_root_component_id,
                component_id,
                text,
            } => self.fresh_chat_baseline.is_some_and(|baseline| {
                crate::proof::fresh_quest_completion_contains(
                    self.snapshot.chat_lines(),
                    self.snapshot.widgets(),
                    baseline,
                    chat_needle,
                    main_root_component_id,
                    component_id,
                    text,
                )
            }),
            Proof::VisitedWithin { .. } => self.visited_within,
            other => other.check_with_xp_context(
                &self.snapshot,
                self.obj_names.as_deref(),
                Some(&self.xp_baselines),
                self.fresh_xp_baseline,
            ),
        }
    }

    /// Restart the style watch's budget at each completed pre-style trip and
    /// each progress-stat XP gain, and fail when the card fights without its
    /// style while it holds food.
    fn observe_loot_trips(&mut self, style: Proof) -> Result<(), String> {
        let StepKind::ObserveStyleAcrossLootTrips {
            deposit_id,
            food,
            progress_stat,
        } = self.current_step().kind
        else {
            return Ok(());
        };
        let progress_xp = self.stat_xp(progress_stat);
        let style_holds = self.holds(style);
        let food_held = self
            .snapshot
            .inv()
            .iter()
            .any(|(id, count)| *count > 0 && food.contains(id));
        let melee_xp = match (self.stat_xp(0), self.stat_xp(2), self.stat_xp(1)) {
            (Some(attack), Some(strength), Some(defence)) => Some([attack, strength, defence]),
            _ => None,
        };
        let fresh_bank = self.snapshot.ingame()
            && self.snapshot.scene_state() == 2
            && self.snapshot.bank_component_id() >= 0
            && self.snapshot.bank_loaded();
        let banked: i32 = if fresh_bank {
            self.snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == deposit_id)
                .map(|row| row.count)
                .sum()
        } else {
            0
        };
        let Some(state) = self.loot_trips.as_mut() else {
            return Ok(());
        };
        let rose = match (state.melee_xp, melee_xp) {
            (Some(before), Some(now)) => before.iter().zip(now).any(|(old, new)| new > *old),
            _ => false,
        };
        if melee_xp.is_some() {
            state.melee_xp = melee_xp;
        }
        let progressed = matches!(
            (state.progress_xp, progress_xp),
            (Some(before), Some(now)) if now > before
        );
        if progress_xp.is_some() {
            state.progress_xp = progress_xp;
        }
        if rose && food_held && !style_holds {
            return Err(format!(
                "melee XP rose while stall food was held and {} did not hold",
                style.name()
            ));
        }
        if banked > state.deposited {
            state.deposited = banked;
            state.trips += 1;
            self.game_ticks_waited = 0;
        }
        if progressed {
            state.steals += 1;
            self.game_ticks_waited = 0;
        }
        Ok(())
    }

    fn observe_combat_bank_returns(&mut self, arm: Proof) -> Result<bool, &'static str> {
        let StepKind::ObserveCombatAcrossBankReturns {
            stand,
            radius,
            max_restarts,
        } = self.current_step().kind
        else {
            return Ok(true);
        };
        let Proof::FreshStatXpGain { id, .. } = arm else {
            return Err("bank-return combat watch requires a fresh XP arm");
        };
        let at_stand = Proof::ArrivedNear {
            x: stand.x,
            z: stand.z,
            level: stand.level,
            radius,
        }
        .check(&self.snapshot, None);
        let fresh_bank = Proof::BankOpen.check(&self.snapshot, None);
        let bank_closed = Proof::BankClosed.check(&self.snapshot, None);
        let bank_generation = self.snapshot.bank_session_generation();
        let xp = self.stat_xp(id);
        let Some(state) = self.combat_bank_returns.as_mut() else {
            return Ok(false);
        };
        if !state.bank_open_seen
            && fresh_bank
            && !at_stand
            && bank_generation > state.bank_generation
        {
            if state.restarts >= max_restarts {
                return Err("bank-return restart limit spent without fresh combat XP");
            }
            state.bank_open_seen = true;
            state.bank_generation = bank_generation;
            self.game_ticks_waited = 0;
        }
        if state.bank_open_seen {
            if bank_closed && at_stand && bank_generation > state.bank_generation {
                state.bank_open_seen = false;
                state.restarts += 1;
                self.fresh_xp_baseline = xp.map(|xp| (id, xp));
                self.game_ticks_waited = 0;
            }
            return Ok(false);
        }
        Ok(bank_closed && at_stand)
    }

    fn observe_stall_combat(&mut self, xp_gained: bool) -> Result<bool, &'static str> {
        let StepKind::ObserveStallCombat { stand, guard_ready } = self.current_step().kind else {
            return Ok(true);
        };
        let bank_open = self.snapshot.bank_component_id() >= 0;
        let state = self
            .stall_combat
            .get_or_insert(StallCombatObservation::FirstSession);
        match state {
            StallCombatObservation::FirstSession => {
                if xp_gained {
                    return Ok(true);
                }
                if bank_open {
                    *state = StallCombatObservation::FirstBank;
                }
            }
            StallCombatObservation::FirstBank => {
                if Proof::BankClosed.check(&self.snapshot, None) {
                    *state = StallCombatObservation::Returning;
                }
            }
            StallCombatObservation::Returning => {
                if (Proof::ArrivedNear {
                    x: stand.x,
                    z: stand.z,
                    level: stand.level,
                    radius: 6,
                })
                .check(&self.snapshot, None)
                {
                    let ready = guard_ready.check(&self.snapshot, self.obj_names.as_deref());
                    *state = StallCombatObservation::SecondSession { guard_ready: ready };
                    return Ok(ready);
                }
            }
            StallCombatObservation::SecondSession { guard_ready: seen } => {
                if !*seen {
                    *seen = guard_ready.check(&self.snapshot, self.obj_names.as_deref());
                }
                if *seen && xp_gained {
                    return Ok(true);
                }
                if bank_open {
                    return Err(
                        "second stall session reached the bank without renewed Guard readiness and combat XP",
                    );
                }
            }
        }
        Ok(false)
    }

    /// Update the current lamp episode from the native host hold input and
    /// this frame's snapshot. Returns `None` for ordinary steps and completion
    /// for the dedicated observation step.
    fn observe_lamp_redemption(&mut self, hold: bool) -> Option<bool> {
        let (lamp_id, reward_stat) = match self.current_step().kind {
            StepKind::ObserveLampRedemption {
                lamp_id,
                reward_stat,
            } => (lamp_id, reward_stat),
            _ => return None,
        };
        let lamp_here = self
            .snapshot
            .inv()
            .iter()
            .any(|(id, count)| *id == lamp_id && *count > 0);
        let reward_xp = self.stat_xp(reward_stat);
        let active_continue =
            Proof::ActiveContinue.check(&self.snapshot, self.obj_names.as_deref());
        let authentic_award = active_continue
            && self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|line| line.contains(LAMP_AWARD_MARKER));
        let state = self
            .lamp_episode
            .get_or_insert_with(LampEpisodeObservation::default);

        if state.reward_baseline.is_none() {
            state.reward_baseline = reward_xp;
        }
        state.lamp_seen |= lamp_here;
        state.hold_seen |= lamp_here && hold;
        if !active_continue && !state.dialogue_seen {
            state.clear_before_dialogue = true;
        }

        let reward_now = state
            .reward_baseline
            .zip(reward_xp)
            .is_some_and(|(before, now)| now > before);
        let consumed_now = state.lamp_seen && !lamp_here;
        state.reward_seen |= reward_now;
        state.consumed_seen |= consumed_now;

        // xplamp_confirm emits multiple packets before mesbox, so the client
        // may publish the durable reward and award continuation on different
        // frames. Bind the continuation to the selected-289 award text and
        // this post-injection held episode instead of requiring an atomic
        // reward/consumption edge. A stale entry dialogue still cannot count.
        if authentic_award && state.clear_before_dialogue && state.lamp_seen && state.hold_seen {
            state.dialogue_seen = true;
        }
        if state.dialogue_seen && !active_continue {
            state.dialogue_drained = true;
        }
        if state.hold_seen
            && state.reward_seen
            && state.consumed_seen
            && state.dialogue_drained
            && !hold
        {
            state.released = true;
        }
        Some(state.released)
    }

    fn observe_maze_completion(&mut self, hold: bool) -> Option<bool> {
        let (spawns, shrine, shrine_radius, min_progress, entry_shot) =
            match self.current_step().kind {
                StepKind::ObserveMazeCompletion {
                    spawns,
                    shrine,
                    shrine_radius,
                    min_progress,
                    entry_shot,
                    ..
                } => (spawns, shrine, shrine_radius, min_progress, entry_shot),
                _ => return None,
            };
        let Some(state) = self.maze_episode.as_mut() else {
            // A disconnect/session boundary clears the episode. Never
            // recapture a post-event tile as a new return baseline.
            return Some(false);
        };
        let tile = snapshot_tile(&self.snapshot);
        let ready = self.snapshot.ingame() && self.snapshot.scene_state() == 2;
        let inv_total = inventory_total(&self.snapshot);
        let mut entry_capture = None;

        if state.entry.is_none() && ready {
            if let Some(outside) = tile.filter(|tile| !on_maze_square(*tile, spawns)) {
                state.return_tile = Some(outside);
                state.reward_baseline = Some(inv_total);
            }
        }
        if state.entry.is_none() && hold {
            if let Some(tile) = tile.filter(|tile| spawns.contains(tile)) {
                state.entry = Some(tile);
                state.last_maze_tile = Some(tile);
                entry_capture = Some(tile);
            }
        }

        if let (Some(entry), Some(here)) = (state.entry, tile) {
            if hold && on_maze_square(here, spawns) {
                let progressed_before = state.progress_seen;
                if state.last_maze_tile != Some(here) {
                    state.tile_changes += 1;
                    state.last_maze_tile = Some(here);
                }
                state.max_progress = state.max_progress.max(tile_distance(entry, here));
                if state.tile_changes >= 2 && state.max_progress >= min_progress {
                    state.progress_seen = true;
                }
                if progressed_before && tile_distance(here, shrine) <= shrine_radius {
                    state.shrine_seen = true;
                }
            }
        }

        if state.shrine_seen
            && ready
            && tile == state.return_tile
            && tile.is_some_and(|tile| !on_maze_square(tile, spawns))
        {
            state.returned = true;
        }
        if state.returned {
            state.reward_seen |= state
                .reward_baseline
                .is_some_and(|baseline| inv_total > baseline);
        }
        if state.returned && state.reward_seen && ready && !hold {
            state.released = true;
        }
        let released = state.released;
        let return_tile = state.return_tile;

        if let Some(entry) = entry_capture {
            println!(
                "MAZE_START: spawn=({},{},{}) return={:?}",
                entry.x, entry.z, entry.level, return_tile
            );
            if let Some(sink) = self.shot_sink.as_mut() {
                sink(entry_shot, &self.snapshot);
            }
        }
        Some(released)
    }

    fn stat_xp(&self, id: i32) -> Option<i32> {
        self.snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == id)
            .map(|stat| stat.xp)
    }

    fn capture_xp_baseline(&mut self, proof: Proof) {
        let id = match proof {
            Proof::StatXpGain { id, .. } => {
                if self.xp_baselines.iter().any(|(i, _)| *i == id) {
                    return;
                }
                id
            }
            Proof::FreshStatXpGain { id, .. } => {
                if !self.snapshot.ingame() || self.fresh_xp_baseline.is_some() {
                    return;
                }
                id
            }
            _ => return,
        };
        let Some(xp) = self
            .snapshot
            .stats()
            .iter()
            .find(|s| s.index == id)
            .map(|s| s.xp)
        else {
            return;
        };
        if matches!(proof, Proof::FreshStatXpGain { .. }) {
            self.fresh_xp_baseline = Some((id, xp));
        } else {
            self.xp_baselines.push((id, xp));
        }
    }

    fn retry_xp_baselines(&mut self) {
        match self.phase {
            Phase::Running => self.capture_xp_baseline(self.current_step().wait.arm),
            Phase::Proving => self.capture_xp_baseline(self.scenario.proof),
            _ => {}
        }
    }

    /// Latch the step-start chat baseline for FreshTutorial, FreshChat,
    /// FreshQuestCompletion, and (when no previous-step baseline was handed
    /// over) FreshChatSincePreviousStep. A stale line at or below this
    /// sequence cannot satisfy the wait.
    fn capture_chat_baseline(&mut self, proof: Proof) {
        if !matches!(
            proof,
            Proof::FreshTutorial
                | Proof::FreshChat { .. }
                | Proof::FreshChatSincePreviousStep { .. }
                | Proof::FreshQuestCompletion { .. }
        ) {
            return;
        }
        if self.fresh_chat_baseline.is_some() {
            return;
        }
        self.fresh_chat_baseline = Some(crate::tutorial::chat_baseline(&self.snapshot));
    }

    /// Latch the baseline the next step will use when its arm is
    /// [`Proof::FreshChatSincePreviousStep`]: the newest chat sequence at
    /// the moment the current (previous) step begins. A burst sent while
    /// this step runs stays above the baseline; history from before this
    /// step does not.
    fn capture_next_chat_baseline(&mut self) {
        if self.next_fresh_chat_baseline.is_some() {
            return;
        }
        let Some(next) = self.scenario.steps.get(self.step + 1) else {
            return;
        };
        if matches!(next.wait.arm, Proof::FreshChatSincePreviousStep { .. }) {
            self.next_fresh_chat_baseline = Some(crate::tutorial::chat_baseline(&self.snapshot));
        }
    }

    /// Latch [`Proof::VisitedWithin`] when a dirty snapshot first shows the
    /// player inside the current step's proof rectangle. The snapshot reader
    /// checks this before it evaluates the step's arm.
    fn observe_visited(&mut self) {
        if self.visited_within {
            return;
        }
        let Proof::VisitedWithin {
            min_x,
            max_x,
            min_z,
            max_z,
            level,
        } = self.current_step().wait.arm
        else {
            return;
        };
        if crate::proof::visited_within_contains(&self.snapshot, min_x, max_x, min_z, max_z, level)
        {
            self.visited_within = true;
        }
    }

    fn retry_chat_baseline(&mut self) {
        if !matches!(self.phase, Phase::Running) {
            return;
        }
        if !self.snapshot.ingame() {
            return;
        }
        self.capture_chat_baseline(self.current_step().wait.arm);
        self.capture_next_chat_baseline();
    }

    fn fire_poll_sustains(&self, client: &mut Client) {
        let names = self.obj_names.as_deref();
        let cheats: Vec<&'static str> = self
            .scenario
            .settings
            .sustains
            .iter()
            .filter_map(|s| match s.when {
                SustainWhen::Poll(p) if p.check(&self.snapshot, names) => Some(s.cheat),
                _ => None,
            })
            .collect();
        for cmd in cheats {
            cheat(client, cmd);
        }
    }

    fn fire_nav_step_sustains(&self, client: &mut Client) {
        if !matches!(
            self.current_step().kind,
            StepKind::Walk { .. } | StepKind::Follow { .. } | StepKind::FollowTele { .. }
        ) {
            return;
        }
        for s in &self.scenario.settings.sustains {
            if matches!(s.when, SustainWhen::EachNavStep) {
                cheat(client, s.cheat);
            }
        }
    }

    fn advance_step(&mut self) {
        self.step += 1;
        if self.step >= self.scenario.steps.len() {
            if !matches!(self.scenario.proof, Proof::FreshStatXpGain { .. }) {
                self.capture_xp_baseline(self.scenario.proof);
            }
            self.phase = Phase::Proving;
        } else {
            self.begin_step();
        }
    }

    fn trace_seed_deposit(&mut self, result: &'static str) {
        let StepKind::NativeSeedDeposit {
            unnoted_id,
            note_id,
            ..
        } = self.current_step().kind
        else {
            return;
        };
        let count = |rows: &[api::snapshot::ItemView], id| {
            rows.iter()
                .filter(|row| row.def.id == id)
                .map(|row| row.count)
                .sum::<i32>()
        };
        let inv_id = note_id.unwrap_or(unnoted_id);
        self.seed_deposits.push(SeedDepositEvidence {
            step: self.step,
            tick: self.snapshot.tick(),
            inv_id,
            result,
            proof: self.holds(self.current_step().wait.arm),
            pack: count(self.snapshot.inventory(), inv_id),
            side: count(self.snapshot.bank_side(), inv_id),
            bank: count(self.snapshot.bank(), unnoted_id),
            main_generation: self.snapshot.bank_snapshot_generation(),
            pack_generation: self.snapshot.inventory_packet_generation(),
            side_generation: self.snapshot.bank_side_packet_generation(),
        });
    }

    /// Send the current step's action once. `Perform` runs its closure;
    /// `Walk` and `Follow` arm the whole-world route (`find_with` over the
    /// collision + transport graph) and drive `Traveller::follow`.
    /// `DrainDialogs` answers the chat modal's choice when one is up, else
    /// closes it (re-sent every client cycle — a refusal with nothing up is
    /// fine); `Shot` sends nothing (the step only waits for its arm to capture
    /// it).
    fn send_current(&mut self, client: &mut Client) -> Result<(), String> {
        match &self.scenario.steps[self.step].kind {
            StepKind::Perform { send } | StepKind::Repeat { send } => {
                if send(client, &self.snapshot) {
                    Ok(())
                } else {
                    Err("driver rejected the send".into())
                }
            }
            StepKind::NativeSeedDeposit {
                unnoted_id,
                quantity,
                note_id,
            } => {
                let result = crate::native_seed_deposit_attempt(
                    client,
                    &self.snapshot,
                    *unnoted_id,
                    *quantity,
                    *note_id,
                );
                let outcome = match result {
                    Ok(crate::SeedDepositProgress::DepositSent) => "DepositSent",
                    Ok(crate::SeedDepositProgress::AlreadyDeposited) => "AlreadyDeposited",
                    Ok(crate::SeedDepositProgress::BankPending) => "BankPending",
                    Err(crate::SeedDepositFailure::MissingRow { .. }) => "MissingRow",
                    Err(crate::SeedDepositFailure::MissingOperation { .. }) => "MissingOperation",
                    Err(crate::SeedDepositFailure::SendRefused { .. }) => "SendRefused",
                };
                self.trace_seed_deposit(outcome);
                match result {
                    Ok(
                        crate::SeedDepositProgress::DepositSent
                        | crate::SeedDepositProgress::AlreadyDeposited,
                    ) => {
                        self.step_sent = true;
                        Ok(())
                    }
                    Ok(crate::SeedDepositProgress::BankPending) => Ok(()),
                    Err(failure) => Err(failure.to_string()),
                }
            }
            StepKind::DrainDialogs { choice } => {
                let mut ix = Interactions::new(&self.snapshot, client);
                if self.snapshot.chat_continue_component_id() != -1 {
                    match ix.continue_dialog(None) {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => Ok(()),
                    }
                } else if !self.snapshot.chat_options().is_empty() {
                    match ix.answer_choice(*choice) {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => Ok(()),
                    }
                } else {
                    let m = self.snapshot.modals();
                    if m.main == -1 && m.side == -1 && m.chat == -1 && m.tutorial == -1 {
                        return Ok(());
                    }
                    match ix.close_modal() {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => Ok(()),
                    }
                }
            }
            StepKind::ObserveMazeCompletion { trigger, .. } => {
                if let Some(cmd) = trigger {
                    if cheat(client, cmd).is_sent() {
                        Ok(())
                    } else {
                        Err("driver rejected the send".into())
                    }
                } else {
                    Ok(())
                }
            }
            StepKind::Shot { .. }
            | StepKind::StartScript
            | StepKind::StopScript
            | StepKind::Await { .. }
            | StepKind::ObserveLampRedemption { .. }
            | StepKind::ObserveStallCombat { .. }
            | StepKind::ObserveStyleAcrossLootTrips { .. }
            | StepKind::ObserveCombatAcrossBankReturns { .. }
            | StepKind::ObserveRandomEvent { .. } => Ok(()),
            StepKind::Relog => {
                self.relog_session.get_or_insert(client.gens.session);
                if client.ingame && !self.relog_logout_sent {
                    let ifaces = std::sync::Arc::clone(&client.ifaces);
                    if !logout(client, ifaces.as_slice()) {
                        return Err(
                            "logout iface missing (side icons still tutorial-locked?)".into()
                        );
                    }
                    self.relog_logout_sent = true;
                }
                Ok(())
            }
            StepKind::Walk { dest } | StepKind::Follow { dest } => {
                self.arm_route(*dest, self.nav_find_opts(false))
            }
            StepKind::FollowTele { dest } => self.arm_route(*dest, self.nav_find_opts(true)),
        }
    }

    fn nav_find_opts(&self, force_teleports: bool) -> FindOptions {
        let n = &self.scenario.settings.nav;
        FindOptions {
            allow_teleports: force_teleports || n.allow_teleports,
            allow_wilderness: n.allow_wilderness,
            zones: if n.allow_danger_zones {
                nav_planner::zones::ZoneExempt::all()
            } else {
                nav_planner::zones::ZoneExempt::NONE
            },
            ..FindOptions::default()
        }
    }

    /// Arm a whole-world route: `nav_planner::router::find` over the packed
    /// collision + transport graph loaded into [`NavWorld`] (the same
    /// surface the route is then walked on — the pack is baked from the
    /// maps, so it matches the live world). The origin is the observed
    /// player tile — a loc-blocked tele landing is fine, the router only
    /// tests tiles stepped *onto*.
    fn arm_route(&mut self, dest: WorldTile, opts: FindOptions) -> Result<(), String> {
        let Some((hx, hz, hl)) = self.snapshot.tile() else {
            return Err("no player tile to route from".into());
        };
        let Some(world) = self.nav_world.as_ref() else {
            return Err("no nav world (run nav-pack); cannot route".into());
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        // The live snapshot's facts gate the route: an edge the player
        // cannot pay / has not earned is not relaxed. The traveller's
        // latched essence-mine session lets a route from inside the mine
        // use the exit portal's return hop to the entry wizard.
        let state = nav_planner::WorldState::from_snapshot(&self.snapshot)
            .with_map_members(self.map_members);
        let opts = FindOptions {
            essence: self.traveller.essence(),
            ..opts
        };
        match nav_planner::router::find_with(
            &world.collision,
            &world.graph,
            from,
            dest,
            opts,
            &state,
        ) {
            Ok(route) => {
                self.route = Some(route);
                Ok(())
            }
            Err(e) => Err(format!(
                "no world path from {from:?} to {dest:?}: {e:?} (essence {:?})",
                opts.essence
            )),
        }
    }

    /// Poll the armed nav route one leg per delivered client-cycle frame
    /// (`dirty` — any family generation moved). `find` already proved the
    /// route at arm time; a terminal outcome other than `Arrived` is a
    /// stall/refusal/block and fails the step with the traveller's own
    /// message. `Arrived` leaves the arm predicate (`Proof::Arrived`) to
    /// fire on the same poll's snapshot read.
    ///
    /// The poll options are a fresh default each call — this runner never
    /// sets an `on_leg` callback, so nothing needs to survive across client
    /// cycles (and a stored `TravelOptions<'static>` would make the runner
    /// non-`Send`, which the panel's shared runner slot requires).
    fn step_route(&mut self, client: &mut Client) {
        let Some(route) = self.route.clone() else {
            return;
        };
        if crate::debug_enabled() {
            let lp = client.local_player.as_ref();
            let phys = lp.map(|p| ((p.x - 64) / 128, (p.z - 64) / 128));
            eprintln!(
                "[nav-runner] game_tick={} snap={:?} phys={:?} dest={:?} legs={} base=({},{}) route={:?} player_gen={} snapshot_game_tick={}",
                self.total_game_ticks,
                self.snapshot.tile(),
                phys,
                route.dest,
                route.legs.len(),
                client.map_build_base_x, client.map_build_base_z,
                lp.map(|p| (p.route_x[0], p.route_z[0])), client.gens.player, self.snapshot.tick(),
            );
        }
        // Exact dest: default close_enough=2 reports Arrived a tile or two
        // short, the proof (exact tile) fails, and the next poll re-starts
        // the original route — walker yo-yos back to the door. The packed
        // teleport list rides along so a jewellery rub hop can answer the
        // destination dialog's choice for its own landing. The options
        // (and their `self.nav_world` borrow) drop inside the block, so
        // the terminal-outcome path below can borrow `self` again.
        let outcome = {
            let mut options = TravelOptions {
                close_enough: 0,
                teleports: self
                    .nav_world
                    .as_ref()
                    .map(|w| w.graph.teleports.as_slice()),
                edges: self.nav_world.as_ref().map(|w| w.graph.edges.as_slice()),
                ..TravelOptions::default()
            };
            self.traveller
                .follow(client, &self.snapshot, route, &mut options)
        };
        let Some(outcome) = outcome else {
            return;
        };
        if crate::debug_enabled() {
            eprintln!("[nav-runner] follow outcome={outcome:?}");
        }
        if let TravelOutcome::Arrived { .. } = outcome {
            // Don't re-arm the original outside→door→dest legs next client cycle.
            self.route = None;
            return;
        }
        self.finish_fail(&format!(
            "step {} ({}): follow {outcome:?}",
            self.step + 1,
            self.current_step().name
        ));
    }

    /// The predicate the terminal evidence names: the failing step's arm,
    /// else the proof.
    fn current_predicate_name(&self) -> String {
        if self.step < self.scenario.steps.len() {
            match &self.scenario.steps[self.step].kind {
                StepKind::Await { evidence, .. } => (*evidence).to_string(),
                _ => self.scenario.steps[self.step].wait.arm.name(),
            }
        } else {
            self.scenario.proof.name()
        }
    }

    /// Fire the armed terminal shot at the terminal state (PASS and FAIL).
    /// The headed panel's sink bridges this to the whole-window readback;
    /// the headless twin's default sink is a no-op.
    fn fire_terminal_shot(&mut self) {
        let Some(label) = self.terminal_shot else {
            return;
        };
        if let Some(sink) = self.shot_sink.as_mut() {
            sink(label, &self.snapshot);
        }
    }

    fn finish_pass(&mut self) {
        if let Some(speed_ms) = self.scenario.settings.teardown_world_speed_ms {
            self.begin_terminal_cleanup(speed_ms, TerminalOutcome::Pass);
            return;
        }
        self.record_pass();
    }

    fn finish_fail(&mut self, msg: &str) {
        if matches!(self.phase, Phase::Cleaning) {
            return;
        }
        if let Some(speed_ms) = self.scenario.settings.teardown_world_speed_ms {
            self.begin_terminal_cleanup(speed_ms, TerminalOutcome::Fail(msg.to_owned()));
            return;
        }
        self.record_fail(msg);
    }

    /// The cell needs an engine test patch this engine lacks: end the run
    /// as UNAVAILABLE (reported as a failed status) naming the patch.
    fn finish_unavailable(&mut self, msg: &str) {
        if matches!(self.phase, Phase::Cleaning) {
            return;
        }
        if let Some(speed_ms) = self.scenario.settings.teardown_world_speed_ms {
            self.begin_terminal_cleanup(speed_ms, TerminalOutcome::Unavailable(msg.to_owned()));
            return;
        }
        self.record_terminal("UNAVAILABLE", msg);
    }

    /// Feed the current frame to the armed random-event episode and send
    /// its trigger when it asks for one. Steps without an episode pass.
    fn observe_random_event(&mut self, client: &mut Client, hold: bool) -> Verdict {
        if !matches!(
            self.current_step().kind,
            StepKind::ObserveRandomEvent { .. }
        ) {
            return Verdict::Pass;
        }
        let Some(episode) = self.random_event.as_mut() else {
            return Verdict::Pass;
        };
        let frame = Frame::from_snapshot(
            &self.snapshot,
            episode.cell(),
            hold,
            self.obj_names.as_deref(),
        );
        if let Some(command) = episode.next_send(&frame) {
            if !cheat(client, &command).is_sent() {
                return Verdict::Fail(format!("driver rejected `{command}`"));
            }
        }
        episode.observe(frame)
    }

    fn begin_terminal_cleanup(&mut self, speed_ms: u32, outcome: TerminalOutcome) {
        self.phase = Phase::Cleaning;
        self.terminal_cleanup = Some(TerminalCleanup {
            speed_ms,
            baseline: None,
            started: Instant::now(),
            outcome,
        });
    }

    fn tick_terminal_cleanup(&mut self, client: &mut Client) {
        const RESET_TIMEOUT: Duration = Duration::from_secs(12);
        let Some((speed_ms, baseline, started)) = self
            .terminal_cleanup
            .as_ref()
            .map(|cleanup| (cleanup.speed_ms, cleanup.baseline, cleanup.started))
        else {
            self.record_fail("world speed cleanup lost its terminal state");
            return;
        };
        if baseline.is_none() {
            let mark = crate::quest_fast::chat_baseline(&self.snapshot);
            if let Some(cleanup) = self.terminal_cleanup.as_mut() {
                cleanup.baseline = Some(mark);
            }
            if let Err(error) = crate::quest_fast::send_tick_speed(client, speed_ms) {
                self.fail_terminal_cleanup(&error);
                return;
            }
            return;
        }
        let baseline = baseline.unwrap();
        if crate::quest_fast::tick_speed_confirmed(&self.snapshot, baseline, speed_ms) {
            let outcome = self.terminal_cleanup.take().unwrap().outcome;
            self.complete_terminal(outcome);
        } else if started.elapsed() >= RESET_TIMEOUT {
            self.fail_terminal_cleanup("speed confirmation timed out");
        }
    }

    fn fail_terminal_cleanup(&mut self, reason: &str) {
        let message = match self.terminal_cleanup.take().map(|cleanup| cleanup.outcome) {
            Some(TerminalOutcome::Pass) => {
                format!("could not restore world speed to 600ms: {reason}")
            }
            Some(TerminalOutcome::Fail(original) | TerminalOutcome::Unavailable(original)) => {
                format!("{original}; additionally could not restore world speed to 600ms: {reason}")
            }
            None => format!("could not restore world speed to 600ms: {reason}"),
        };
        self.record_fail(&message);
    }

    fn complete_terminal(&mut self, outcome: TerminalOutcome) {
        match outcome {
            TerminalOutcome::Pass => self.record_pass(),
            TerminalOutcome::Fail(message) => self.record_fail(&message),
            TerminalOutcome::Unavailable(message) => self.record_terminal("UNAVAILABLE", &message),
        }
    }

    fn log_tick_speed_endpoint(&self, speed_ms: u32) {
        match self.tick_speed_endpoint.as_ref() {
            Some((host, port)) => {
                eprintln!("world-wide ::speed {speed_ms}ms target endpoint={host}:{port}");
            }
            None => {
                eprintln!("world-wide ::speed {speed_ms}ms target endpoint=unknown");
            }
        }
    }

    fn observe_ground_guard(&mut self, arm: Proof) {
        if !matches!(arm, Proof::GroundItemIdsAbsentNear { .. }) {
            return;
        }
        let Some(guard) = self.ground_guard.as_mut() else {
            return;
        };
        guard.ticks_waited = self.snapshot.tick().wrapping_sub(guard.start_tick);
        for item in self
            .snapshot
            .ground_items()
            .iter()
            .filter(|item| arm.ground_guard_contains(item))
        {
            let tile = [item.tile.x, item.tile.z, item.tile.level];
            if let Some(seen) = guard
                .items_seen
                .iter_mut()
                .find(|seen| seen.id == item.def.id && seen.tile == tile)
            {
                seen.count = seen.count.max(item.count);
            } else {
                guard.items_seen.push(GroundGuardItem {
                    id: item.def.id,
                    count: item.count,
                    tile,
                });
            }
        }
    }

    /// The read-only terminal summary of the most recent Await step
    /// among the first `reached` steps, if that step carried one.
    /// On a pass every step was reached. On a failure the steps up
    /// to and including the failing step were reached. The summary
    /// rides the receipt either way, so a failure names its unmet
    /// witness terms and a pass still records the summary's
    /// observation values. An empty summary records nothing.
    fn reached_terminal_summary(&self, reached: usize) -> Option<String> {
        self.scenario.steps[..reached.min(self.scenario.steps.len())]
            .iter()
            .rev()
            .find_map(|step| match &step.kind {
                StepKind::Await {
                    summary: Some(summary),
                    ..
                } => Some(summary()),
                _ => None,
            })
            .filter(|summary| !summary.is_empty())
    }

    fn record_pass(&mut self) {
        self.phase = Phase::Done;
        self.fire_terminal_shot();
        let mut evidence = Evidence::terminal(
            self.scenario.name,
            "PASS",
            self.scenario.proof.name(),
            self.reached_terminal_summary(self.scenario.steps.len()),
            self.total_game_ticks,
            &self.snapshot,
            self.obj_names.as_deref(),
            self.started,
        );
        evidence.receipt = self.script_receipts.first().cloned();
        evidence.protect_window = self
            .protect_window
            .as_ref()
            .map(ProtectWindowObservation::evidence);
        evidence.random_event = self.random_event.as_ref().map(RandomEventEpisode::receipt);
        evidence.ground_guard = self.ground_guard.take();
        evidence.seed_deposits = std::mem::take(&mut self.seed_deposits);
        self.evidence = Some(evidence);
    }

    fn record_fail(&mut self, msg: &str) {
        self.record_terminal("FAIL", msg);
    }

    fn record_terminal(&mut self, outcome: &'static str, msg: &str) {
        self.phase = Phase::Done;
        self.fire_terminal_shot();
        // The most recent reached Await may carry a read-only
        // terminal summary of what it still waits for. Append it so
        // the receipt names the unmet terms on budget, deadline and
        // later-step failures alike.
        let msg = match self.reached_terminal_summary(self.step + 1) {
            Some(summary) => format!("{msg} [{summary}]"),
            None => msg.to_string(),
        };
        let mut evidence = Evidence::terminal(
            self.scenario.name,
            outcome,
            self.current_predicate_name(),
            Some(msg),
            self.total_game_ticks,
            &self.snapshot,
            self.obj_names.as_deref(),
            self.started,
        );
        evidence.receipt = self.script_receipts.first().cloned();
        evidence.protect_window = self
            .protect_window
            .as_ref()
            .map(ProtectWindowObservation::evidence);
        evidence.random_event = self.random_event.as_ref().map(RandomEventEpisode::receipt);
        evidence.ground_guard = self.ground_guard.take();
        evidence.seed_deposits = std::mem::take(&mut self.seed_deposits);
        self.evidence = Some(evidence);
    }
}

/// The distinct receipt prefixes a scenario's [`Proof::ScriptReceipt`]
/// watches name.
fn receipt_prefixes(scenario: &Scenario) -> Vec<&'static str> {
    let mut prefixes = Vec::new();
    for proof in scenario
        .steps
        .iter()
        .map(|step| step.wait.arm)
        .chain([scenario.proof])
    {
        if let Proof::ScriptReceipt { prefix } = proof {
            if !prefixes.contains(&prefix) {
                prefixes.push(prefix);
            }
        }
    }
    prefixes
}

fn snapshot_tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn inventory_total(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .inv()
        .iter()
        .map(|(_, count)| (*count).max(0))
        .sum()
}

fn on_maze_square(tile: WorldTile, spawns: &[WorldTile]) -> bool {
    spawns.first().is_some_and(|spawn| {
        tile.level == spawn.level && (tile.x >> 6, tile.z >> 6) == (spawn.x >> 6, spawn.z >> 6)
    })
}

fn tile_distance(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        return i32::MAX;
    }
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

/// A `Wait` helper so tests can build steps without importing the field
/// order.
#[cfg(test)]
fn wait(arm: Proof, budget_ticks: u32) -> Wait {
    Wait { arm, budget_ticks }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
