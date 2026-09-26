//! The shared scenario runner: a state machine both runners pump from
//! their own per-frame hooks. No sleeps inside — each `tick` is one
//! observe of the client (the panel slot thread headed, the
//! `run_with_io` hook headless), so "one scenario, two runners" shares
//! the exact same seed/step/proof logic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::{cheat, logout, Interactions, SendResult};
use api::obj_names::ObjNames;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav::router::{FindOptions, Route};
use nav::traveller::{TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;

use crate::evidence::Evidence;
use crate::proof::Proof;
use crate::{Scenario, StepKind, SustainWhen, Wait};

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
    Done,
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

const LAMP_AWARD_MARKER: &str = "Your wish has been granted!";

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
    /// The armed nav route (re-passed each tick; the traveller consumes
    /// it when a run starts).
    route: Option<Route>,
    /// Whole-window shot label fired at the terminal state (PASS and
    /// FAIL); `None` for scenarios that only fire `Shot` steps.
    terminal_shot: Option<&'static str>,
    phase: Phase,
    step: usize,
    step_sent: bool,
    /// Game ticks waited on the current step's arm (post-send observes
    /// where any family gen moved).
    ticks_waited: u32,
    /// Ticks across the whole run (the evidence `ticks` field).
    total_ticks: u32,
    started: Instant,
    deadline: Duration,
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
    engine_speed_ms: Option<u32>,
    engine_speed_sent: bool,
    evidence: Option<Evidence>,
    /// Skill XP baselines captured when a watch step with [`Proof::StatXpGain`]
    /// begins — `(skill id, xp at step start)`.
    xp_baselines: Vec<(i32, i32)>,
    /// Baseline for the current [`Proof::FreshStatXpGain`] step only. Cleared
    /// at every step and session boundary so prior work cannot satisfy it.
    fresh_xp_baseline: Option<(i32, i32)>,
    /// Native host-hold + snapshot facts for the current lamp witness step.
    /// This is deliberately not part of [`GameSnapshot`].
    lamp_episode: Option<LampEpisodeObservation>,
    /// Native host-hold + snapshot ordering for the current Maze witness.
    maze_episode: Option<MazeEpisodeObservation>,
    /// Whole-window shot sink: fired once when a `StepKind::Shot` step's
    /// arm holds, with the label and the terminal snapshot. The headed
    /// panel fills this with its window capture; the headless twin keeps
    /// the default no-op.
    #[allow(clippy::type_complexity)]
    shot_sink: Option<Box<dyn FnMut(&str, &GameSnapshot) + Send>>,
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

    fn with_data(scenario: Scenario, nav_world: Option<Arc<NavWorld>>) -> Self {
        // Settings fields are Copy, so read them out before `scenario`
        // moves into the runner below.
        let deadline = scenario.settings.deadline;
        let terminal_shot = scenario.settings.terminal_shot;
        let require_mainland_base =
            scenario.seed.mainland || scenario.settings.require_mainland_base;
        let engine_speed_ms = scenario.settings.nav.engine_speed_ms;
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
            ticks_waited: 0,
            total_ticks: 0,
            started: Instant::now(),
            deadline,
            scene_settle: Duration::from_secs(2),
            scene2_since: None,
            seed_ready_observed: false,
            seed_arrival_observed: false,
            require_mainland_base,
            live_names: Vec::new(),
            relog_logout_sent: false,
            engine_speed_ms,
            engine_speed_sent: false,
            evidence: None,
            xp_baselines: Vec::new(),
            fresh_xp_baseline: None,
            lamp_episode: None,
            maze_episode: None,
            shot_sink: None,
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
    /// live name when armed); per-frame hooks tick only this slot's client.
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

    /// The companion slot whose profile name is `name`, `None` when no
    /// companion carries it. The driven slot (profile 0) is never a
    /// companion, so per-frame hooks dispatch: driven → [`ScenarioRunner::tick`],
    /// else `companion_for` → [`ScenarioRunner::companion_tick`].
    pub fn companion_for(&self, name: &str) -> Option<usize> {
        self.scenario
            .companions
            .iter()
            .position(|c| self.seed_name(c.profile) == name)
    }

    /// Run companion `index`'s per-frame hook once on `client`. No sleeps;
    /// the caller delivers one frame per call, exactly like [`tick`].
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

    /// The terminal evidence record, `None` until PASS/FAIL.
    pub fn evidence(&self) -> Option<&Evidence> {
        self.evidence.as_ref()
    }

    /// The runner's pollable status.
    pub fn status(&self) -> RunnerStatus {
        match self.phase {
            Phase::Done => match self.evidence.as_ref().map(|e| e.outcome) {
                Some("FAIL") => RunnerStatus::Failed(
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
            Phase::Proving => RunnerStatus::Running {
                step: self.scenario.steps.len(),
                total: self.scenario.steps.len(),
            },
        }
    }

    /// One observe of `client`: rebuild the snapshot, send the current
    /// step's action if it has not gone out yet, hop the traveller, and
    /// check the step arm / proof predicate. Never sleeps. A seeding-
    /// completion tick falls through into the run logic, so the send goes
    /// out the tick the seed completes.
    pub fn tick(&mut self, client: &mut Client) {
        self.tick_with_hold(client, false);
    }

    /// Like [`ScenarioRunner::tick`], but skip `Traveller::follow` while
    /// `hold` is set (guardian hold — the armed route stays latched).
    pub fn tick_with_hold(&mut self, client: &mut Client, hold: bool) {
        self.tick_inner(client, hold);
        if matches!(self.phase, Phase::Done) {
            // A follow failure may still be consumed by this tick's remaining
            // checks. Release only after all existing callbacks and evidence.
            self.snapshot = GameSnapshot::new();
        }
    }

    fn tick_inner(&mut self, client: &mut Client, hold: bool) {
        if matches!(self.phase, Phase::Done) {
            return;
        }
        let dirty = self.snapshot.rebuild(client);
        if !self.snapshot.ingame() {
            self.fresh_xp_baseline = None;
            self.lamp_episode = None;
            self.maze_episode = None;
        }
        self.retry_xp_baselines();
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
            if was_ready && self.snapshot.tile() == Some((3220, 3220, 0)) {
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
                    cheat(client, &format!("speed {ms}"));
                }
                self.engine_speed_sent = true;
            }
            self.fire_poll_sustains(client);
            // Guardian hold freezes follow only; sends/arms still run so a
            // route armed under hold resumes when it lifts.
            if dirty && !hold {
                self.step_route(client);
            }
            if !self.step_sent {
                self.fire_nav_step_sustains(client);
                if let Err(msg) = self.send_current(client) {
                    self.finish_fail(&format!(
                        "step {} ({}): {msg}",
                        self.step + 1,
                        self.current_step().name
                    ));
                    return;
                }
                // A `DrainDialogs` janitor re-sends every tick: it answers
                // whatever `p_choice` dialog is up (the quest-seed
                // prompts) and closes the quest-scroll modals until its
                // arm holds, so the send stays armed — and the step's
                // tick budget keeps counting.
                self.step_sent = !matches!(
                    self.current_step().kind,
                    StepKind::DrainDialogs { .. } | StepKind::Relog | StepKind::Repeat { .. }
                );
                if self.step_sent {
                    self.ticks_waited = 0;
                }
            }
            if dirty {
                self.ticks_waited += 1;
                self.total_ticks += 1;
            }
            let (arm, budget) = {
                let wait = &self.current_step().wait;
                (wait.arm, wait.budget_ticks)
            };
            let native_episode_holds = self.observe_lamp_redemption(hold).unwrap_or(true)
                && self.observe_maze_completion(hold).unwrap_or(true);
            if native_episode_holds
                && arm.check_with_xp_context(
                    &self.snapshot,
                    self.obj_names.as_deref(),
                    Some(&self.xp_baselines),
                    self.fresh_xp_baseline,
                )
            {
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
            } else if self.ticks_waited >= budget {
                let missing = self
                    .lamp_episode
                    .as_ref()
                    .map(LampEpisodeObservation::summary)
                    .or_else(|| {
                        self.maze_episode
                            .as_ref()
                            .map(MazeEpisodeObservation::summary)
                    })
                    .unwrap_or_else(|| arm.name());
                self.finish_fail(&format!(
                    "step {} ({}): {} not seen within {} ticks",
                    self.step + 1,
                    self.current_step().name,
                    missing,
                    budget
                ));
            }
        }
        if matches!(self.phase, Phase::Proving)
            && self.scenario.proof.check_with_xp_context(
                &self.snapshot,
                self.obj_names.as_deref(),
                Some(&self.xp_baselines),
                self.fresh_xp_baseline,
            )
        {
            self.finish_pass();
        }
        if !matches!(self.phase, Phase::Done) && self.started.elapsed() > self.deadline {
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
        self.relog_logout_sent = false;
        self.ticks_waited = 0;
        self.traveller.clear();
        self.route = None;
        self.fresh_xp_baseline = None;
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
        self.capture_xp_baseline(self.current_step().wait.arm);
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

    /// Send the current step's action once. `Perform` runs its closure;
    /// `Walk` and `Follow` both arm the whole-world route (`find_with`
    /// over the collision + transport graph, default options) and drive
    /// `Traveller::follow`; `DrainDialogs` answers the chat modal's
    /// choice when one is up, else closes the open modal (re-sent every
    /// tick by the runner — a refusal with nothing up is fine); `Shot`
    /// sends nothing (the step only waits for its arm to capture it).
    fn send_current(&mut self, client: &mut Client) -> Result<(), String> {
        match &self.scenario.steps[self.step].kind {
            StepKind::Perform { send } | StepKind::Repeat { send } => {
                if send(client, &self.snapshot) {
                    Ok(())
                } else {
                    Err("driver rejected the send".into())
                }
            }
            StepKind::DrainDialogs { choice } => {
                let mut ix = Interactions::new(&self.snapshot, client);
                if self.snapshot.chat_continue_component_id() != -1 {
                    match ix.continue_dialog() {
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
                    if cheat(client, cmd) {
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
            | StepKind::ObserveLampRedemption { .. } => Ok(()),
            StepKind::Relog => {
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
            ..FindOptions::default()
        }
    }

    /// Arm a whole-world route: `nav::router::find` over the packed
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
        let state =
            nav::WorldState::from_snapshot(&self.snapshot).with_map_members(self.map_members);
        let opts = FindOptions {
            essence: self.traveller.essence(),
            ..opts
        };
        match nav::router::find_with(&world.collision, &world.graph, from, dest, opts, &state) {
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

    /// Poll the armed nav route one leg per delivered frame (`dirty` —
    /// any family gen moved). `find` already proved the route at arm
    /// time; a terminal outcome other than `Arrived` is a
    /// stall/refusal/block and fails the step with the traveller's own
    /// message. `Arrived` leaves the arm predicate (`Proof::Arrived`) to
    /// fire on the same tick's snapshot read.
    ///
    /// The poll options are a fresh default each call — this runner never
    /// sets an `on_leg` callback, so nothing needs to survive across ticks
    /// (and a stored `TravelOptions<'static>` would make the runner
    /// non-`Send`, which the panel's shared runner slot requires).
    fn step_route(&mut self, client: &mut Client) {
        let Some(route) = self.route.clone() else {
            return;
        };
        if crate::debug_enabled() {
            let lp = client.local_player.as_ref();
            let phys = lp.map(|p| ((p.x - 64) / 128, (p.z - 64) / 128));
            eprintln!(
                "[nav-runner] tick={} snap={:?} phys={:?} dest={:?} legs={} base=({},{}) route={:?} player_gen={} snapshot_tick={}",
                self.total_ticks,
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
            // Don't re-arm the original outside→door→dest legs next tick.
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
            self.scenario.steps[self.step].wait.arm.name()
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
        self.phase = Phase::Done;
        self.fire_terminal_shot();
        self.evidence = Some(Evidence::terminal(
            self.scenario.name,
            "PASS",
            self.scenario.proof.name(),
            None,
            self.total_ticks,
            &self.snapshot,
            self.obj_names.as_deref(),
            self.started,
        ));
    }

    fn finish_fail(&mut self, msg: &str) {
        self.phase = Phase::Done;
        self.fire_terminal_shot();
        self.evidence = Some(Evidence::terminal(
            self.scenario.name,
            "FAIL",
            self.current_predicate_name(),
            Some(msg.to_string()),
            self.total_ticks,
            &self.snapshot,
            self.obj_names.as_deref(),
            self.started,
        ));
    }
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
#[allow(dead_code)]
fn wait(arm: Proof, budget_ticks: u32) -> Wait {
    Wait { arm, budget_ticks }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
