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
        if authentic_award
            && state.clear_before_dialogue
            && state.lamp_seen
            && state.hold_seen
        {
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
            StepKind::Shot { .. }
            | StepKind::StartScript
            | StepKind::ObserveLampRedemption { .. }
            | StepKind::ObserveMazeCompletion { .. } => Ok(()),
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
mod tests {
    use super::*;
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfTypeMut};
    use client::config::IfType;
    use client::dash3d::ClientPlayer;
    use client::io::{ClientProt, ServerProt};
    use nav::grid::StepGrid;
    use nav::tile::Tile;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::{Companion, Scenario, ScenarioSettings, Seed, Step, StepKind, Sustain};

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A synthetic client that has already seeded: ingame, scene 2, a
    /// mainland build base, and the family gens bumped so the first
    /// rebuild counts as a tick.
    fn seeded_client() -> Client {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 20));
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::REBUILD_NORMAL,
            ServerProt::UPDATE_STAT,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    fn stat_scenario(min: i32, budget: u32) -> Scenario {
        Scenario {
            name: "run-energy",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "set run energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 99;
                        true
                    }),
                },
                wait: wait(Proof::Stat { id: 16, min }, budget),
            }],
            proof: Proof::Stat { id: 16, min },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    #[test]
    fn wait_script_stop_is_none_unless_the_scenario_sets_it() {
        let v1 = ScenarioRunner::new(crate::get("bone_burier").expect("v1"));
        assert_eq!(v1.wait_script_stop(), None);
        let v2 = ScenarioRunner::new(crate::get("bone_burier_v2_ts").expect("v2"));
        assert_eq!(
            v2.wait_script_stop(),
            Some("confirmed loaded current-generation bank exhaustion")
        );
    }

    #[test]
    fn terminal_snapshot_is_released_after_shot_and_evidence() {
        for failed in [false, true] {
            let mut client = seeded_client();
            client.runenergy = 73;
            let mut runner = ScenarioRunner::with_world(stat_scenario(1, 10), None);
            runner.snapshot.rebuild(&client);
            let terminal = serde_json::to_value(&runner.snapshot).unwrap();
            let empty = serde_json::to_value(GameSnapshot::new()).unwrap();
            assert_ne!(terminal, empty);
            let shots = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&shots);
            runner.set_terminal_shot("terminal");
            runner.set_shot_sink(Box::new(move |label, snapshot| {
                captured
                    .lock()
                    .unwrap()
                    .push((label.to_owned(), serde_json::to_value(snapshot).unwrap()));
            }));
            if failed {
                runner.finish_fail("original failure");
            } else {
                runner.finish_pass();
            }
            assert_eq!(
                *shots.lock().unwrap(),
                vec![("terminal".to_owned(), terminal)]
            );
            let evidence = runner.evidence().unwrap();
            assert_eq!(evidence.tile, Some([3220, 3220, 0]));
            assert_eq!(evidence.scene, 2);
            assert_eq!(evidence.stat.as_ref().unwrap().runenergy, 73);
            assert_eq!(evidence.outcome, if failed { "FAIL" } else { "PASS" });
            assert_eq!(
                evidence.message.as_deref(),
                failed.then_some("original failure")
            );
            let retained_evidence = evidence.to_json();
            let retained_status = format!("{:?}", runner.status());
            runner.tick_with_hold(&mut client, false);
            assert_eq!(serde_json::to_value(&runner.snapshot).unwrap(), empty);
            client.runenergy = 1;
            client.ingame = false;
            runner.tick_with_hold(&mut client, true);
            runner.tick(&mut client);
            assert_eq!(runner.evidence().unwrap().to_json(), retained_evidence);
            assert_eq!(format!("{:?}", runner.status()), retained_status);
            assert_eq!(shots.lock().unwrap().len(), 1);
            assert_eq!(serde_json::to_value(&runner.snapshot).unwrap(), empty);
        }
    }

    #[test]
    fn follow_failure_keeps_snapshot_for_existing_same_tick_budget_failure() {
        let mut client = seeded_client();
        let mut runner = ScenarioRunner::with_world(stat_scenario(999, 1), None);
        runner.set_scene_settle(Duration::ZERO);
        runner.phase = Phase::Running;
        runner.step_sent = true;
        runner.route = Some(Route {
            dest: WorldTile {
                x: 3221,
                z: 3220,
                level: 0,
            },
            legs: vec![nav::router::Leg::Walk {
                tiles: vec![
                    WorldTile {
                        x: 3220,
                        z: 3220,
                        level: 0,
                    },
                    WorldTile {
                        x: 3221,
                        z: 3220,
                        level: 0,
                    },
                ],
            }],
            ticks: 1.0,
        });
        let shots = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&shots);
        runner.set_terminal_shot("failure");
        runner.set_shot_sink(Box::new(move |_, snap| {
            captured.lock().unwrap().push(snap.tile());
        }));
        runner.tick(&mut client);
        // Existing behavior: follow refusal does not short-circuit the budget
        // check. Memory cleanup must not change either callback's snapshot.
        assert_eq!(*shots.lock().unwrap(), vec![Some((3220, 3220, 0)); 2]);
        let evidence = runner.evidence().unwrap();
        assert_eq!(evidence.tile, Some([3220, 3220, 0]));
        assert!(evidence
            .message
            .as_ref()
            .unwrap()
            .contains("not seen within 1 ticks"));
    }

    #[test]
    fn new_reads_deadline_and_terminal_shot_from_settings() {
        let runner = ScenarioRunner::new(crate::get("nav_full").unwrap());
        assert_eq!(runner.deadline(), Duration::from_secs(360));
        assert_eq!(runner.terminal_shot(), Some("nav_full terminal"));
    }

    #[test]
    fn hold_skips_follow_and_keeps_the_armed_route() {
        use nav::router::Route;

        let mut c = seeded_client();
        let dest = WorldTile {
            x: 4,
            z: 0,
            level: 0,
        };
        let mut runner = ScenarioRunner::new(Scenario {
            name: "hold-follow",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "walk",
                kind: StepKind::Walk { dest },
                wait: wait(
                    Proof::Arrived {
                        x: 4,
                        z: 0,
                        level: 0,
                    },
                    100,
                ),
            }],
            proof: Proof::Arrived {
                x: 4,
                z: 0,
                level: 0,
            },
            companions: vec![],
            settings: ScenarioSettings::default(),
        });
        runner.set_scene_settle(Duration::ZERO);
        // Skip seeding: force Running with an already-armed route.
        runner.phase = Phase::Running;
        runner.route = Some(Route {
            dest,
            legs: vec![],
            ticks: 0.0,
        });
        runner.step_sent = true;
        runner.tick_with_hold(&mut c, true);
        assert!(
            runner.route.is_some(),
            "hold must not consume the latched scenario route"
        );
    }

    #[test]
    fn new_leaves_the_mainland_base_gate_off_by_default() {
        let mut c = Client::new(cfg());
        let mut runner = ScenarioRunner::new(stat_scenario(1, 10));
        runner.set_scene_settle(Duration::ZERO);
        c.ingame = true;
        c.scene_state = 2;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "default gate off: scene 2 on a tutorial-scale base starts steps"
        );
    }

    #[test]
    fn perform_send_then_arm_fires_and_proof_passes() {
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(stat_scenario(1, 10));
        runner.set_scene_settle(Duration::ZERO);
        assert_eq!(runner.status(), RunnerStatus::Seeding);
        // Tick 1: seed completes, send runs (runenergy 99 lands on the
        // client), the stale snapshot still shows 0.
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
        // Tick 2: the stat bump refreshes the snapshot; the arm fires and
        // the proof passes on the same tick.
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.outcome, "PASS");
        assert_eq!(ev.predicate, "stat(16)>=1");
        assert_eq!(ev.tile, Some([3220, 3220, 0]));
    }

    #[test]
    fn perform_budget_fails_when_the_arm_never_fires() {
        let mut c = seeded_client();
        // The no-op send leaves runenergy at 0; min 999 never holds.
        let mut runner = ScenarioRunner::new(stat_scenario(999, 3));
        runner.set_scene_settle(Duration::ZERO);
        for _ in 0..5 {
            c.bump_gens(ServerProt::UPDATE_RUNENERGY);
            runner.tick(&mut c);
        }
        match runner.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("not seen within 3 ticks"),
                    "fail message names the arm and budget: {msg}"
                );
                assert!(msg.contains("stat(16)>=999"), "names the arm: {msg}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let ev = runner.evidence().expect("evidence at FAIL");
        assert_eq!(ev.outcome, "FAIL");
        assert_eq!(ev.predicate, "stat(16)>=999");
        assert!(ev.message.is_some());
    }

    #[test]
    fn poll_sustain_sends_the_cheat_when_energy_is_at_or_below_the_line() {
        let mut scenario = stat_scenario(1, 10);
        scenario.settings.sustains = vec![Sustain::poll(
            Proof::StatAtMost { id: 16, max: 25 },
            "~energy",
        )];
        let mut c = seeded_client();
        c.runenergy = 10;
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        assert_eq!(
            c.out.data().first().copied(),
            Some(ClientProt::CLIENT_CHEAT.id as u8),
            "~energy is CLIENT_CHEAT, not a host set_run"
        );
    }

    #[test]
    fn seeding_waits_for_ingame_scene2_and_mainland_base() {
        let mut c = Client::new(cfg());
        // The gate is opt-in: this test exercises it, so the scenario
        // turns it on. `new_leaves_the_mainland_base_gate_off_by_default`
        // covers the default-off path.
        let mut scenario = stat_scenario(1, 10);
        scenario.settings.require_mainland_base = true;
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Seeding);

        // Ingame with a tutorial-scale base must still hold.
        c.ingame = true;
        c.scene_state = 2;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Seeding,
            "a sub-3000 base is still seeding (tutorial island)"
        );

        // The queued hop may leave the initial ready frame.
        c.scene_state = 1;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Seeding);

        // A mainland build base and the requested landing release the seed.
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 20));
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    }

    #[test]
    fn mainland_seed_requires_a_completed_hop_before_releasing() {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        // The observed tutorial frame can carry a mainland-looking build
        // base and a tile inside the full-world pack.
        c.map_build_base_x = 3072;
        c.map_build_base_z = 3072;
        c.local_player = Some(ClientPlayer::at(22, 34));
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        let mut scenario = stat_scenario(1, 10);
        scenario.seed.mainland = true;
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Seeding);

        // An unrelated rebuild on the tutorial island is not the requested
        // mainland arrival, even though its base lies inside the full pack.
        c.local_player = Some(ClientPlayer::at(20, 20));
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Seeding);

        // The actual mainland landing releases the seed, without requiring
        // an intermediate scene_state != 2 frame.
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    }

    // --- Whole-world `Walk` steps (find + Traveller::follow) ---

    fn walk_scenario(dest: WorldTile, budget: u32) -> Scenario {
        Scenario {
            name: "walk",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "walk the corridor",
                kind: StepKind::Walk { dest },
                wait: wait(
                    Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget,
                ),
            }],
            proof: Proof::Arrived {
                x: dest.x,
                z: dest.z,
                level: dest.level,
            },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    /// A `Walk` step routes exactly like `Follow`: `nav::router::find`
    /// over the whole-world `NavWorld` (collision + transport graph),
    /// driven by `Traveller::follow` one leg per delivered frame.
    #[test]
    fn walk_step_arms_find_and_proofs_arrival() {
        let mut c = follow_client();
        let dest = WorldTile {
            x: 3205,
            z: 3230,
            level: 0,
        };
        let grid = StepGrid::fixture_rect_at(
            Tile {
                x: 3205,
                z: 3200,
                level: 0,
            },
            1,
            40,
        );
        let world = NavWorld::from_grid(&grid);
        let mut runner =
            ScenarioRunner::with_world(walk_scenario(dest, 120), Some(Arc::new(world)));
        runner.set_scene_settle(Duration::ZERO);

        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "send armed the whole-world route"
        );
        assert!(
            runner.armed_route().is_some(),
            "the headed paint reads this route as the red baked path"
        );

        // Advance the player north one tile per tick; the follow polls its
        // settle on every delivered frame and the arm fires on the exact
        // destination tile.
        let mut steps = 0;
        while runner.status() != RunnerStatus::Passed {
            if steps > 80 {
                panic!("walk never arrived; status={:?}", runner.status());
            }
            steps += 1;
            c.local_player = Some(ClientPlayer::at(5, steps));
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut c);
        }
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.tile, Some([3205, 3230, 0]));
        assert_eq!(ev.predicate, "arrived(3205,3230,0)");
        assert!(ev.ticks > 0, "the walk counted delivered frames");
    }

    #[test]
    fn walk_step_fails_without_a_nav_world() {
        let mut c = follow_client();
        let dest = WorldTile {
            x: 3205,
            z: 3230,
            level: 0,
        };
        let scenario = walk_scenario(dest, 120);
        // `with_world(None)` injects no router world: the walk step must
        // fail with a clear pack message.
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        match runner.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("no nav world"),
                    "clear world error names the pack: {msg}"
                )
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // --- Whole-world `Follow` steps (find + Traveller::follow) ---

    /// A synthetic client with a connected stream (so the snapshot is
    /// `attached` and `Interactions::walk` passes its preconditions),
    /// seeded on a mainland base, standing at world (3205, 3200) — scene
    /// (5, 0), off the fresh client's `_BOUNDS` border columns.
    fn follow_client() -> Client {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port())
            .expect("connect");
        // Keep the listener alive so the connect stays established.
        std::mem::forget(listener);
        let mut c = Client::new(cfg());
        c.stream = Some(stream);
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(5, 0));
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::REBUILD_NORMAL,
            ServerProt::UPDATE_STAT,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    fn follow_scenario(dest: WorldTile, budget: u32) -> Scenario {
        Scenario {
            name: "follow",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "follow the corridor",
                kind: StepKind::Follow { dest },
                wait: wait(
                    Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget,
                ),
            }],
            proof: Proof::Arrived {
                x: dest.x,
                z: dest.z,
                level: dest.level,
            },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    /// A 1×40 corridor along z at x=3205 (scene x=5, off the fresh
    /// client's `_BOUNDS` border columns): the route is forced, so the
    /// follow's hops cannot wander like a 0-cost Dijkstra does on an open
    /// plane. The `find`-then-`follow` mechanics are what is under test.
    #[test]
    fn follow_step_arms_find_and_proofs_arrival() {
        let mut c = follow_client();
        let dest = WorldTile {
            x: 3205,
            z: 3230,
            level: 0,
        };
        let grid = StepGrid::fixture_rect_at(
            Tile {
                x: 3205,
                z: 3200,
                level: 0,
            },
            1,
            40,
        );
        let world = NavWorld::from_grid(&grid);
        let mut runner =
            ScenarioRunner::with_world(follow_scenario(dest, 120), Some(Arc::new(world)));
        runner.set_scene_settle(Duration::ZERO);

        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "send armed the whole-world route"
        );

        // Advance the player north one tile per tick; the follow polls its
        // settle on every delivered frame and the arm fires on the exact
        // destination tile.
        let mut steps = 0;
        while runner.status() != RunnerStatus::Passed {
            if steps > 80 {
                panic!("follow never arrived; status={:?}", runner.status());
            }
            steps += 1;
            c.local_player = Some(ClientPlayer::at(5, steps));
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut c);
        }
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.tile, Some([3205, 3230, 0]));
        assert_eq!(ev.predicate, "arrived(3205,3230,0)");
        assert!(ev.ticks > 0, "the walk counted delivered frames");
    }

    #[test]
    fn follow_step_fails_without_a_nav_world() {
        let mut c = follow_client();
        let dest = WorldTile {
            x: 3205,
            z: 3230,
            level: 0,
        };
        // `with_world(None)` injects no router world.
        let mut runner = ScenarioRunner::with_world(follow_scenario(dest, 120), None);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        match runner.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("no nav world"),
                    "clear world error names the pack: {msg}"
                )
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // --- Dialog-janitor steps (`DrainDialogs`) ---

    /// A `DrainDialogs` step whose arm never holds: the fixture has no
    /// chat modal, so every re-send is a harmless refusal. The step must
    /// keep re-sending each tick (never count as "already sent") and fail
    /// on the tick budget, not on the first send.
    #[test]
    fn drain_dialogs_resends_every_tick_and_fails_on_the_budget() {
        let scenario = Scenario {
            name: "drain",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "drain",
                kind: StepKind::DrainDialogs { choice: 1 },
                wait: wait(Proof::Stat { id: 16, min: 999 }, 3),
            }],
            proof: Proof::Stat { id: 16, min: 0 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut c = follow_client();
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        // Tick 1-4: the janitor re-sends each tick (no chat choice on the
        // fixture -> refused, which is fine) and the budget lapses.
        for _ in 0..4 {
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut c);
        }
        match runner.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("not seen within 3 ticks"),
                    "the drain step fails on its tick budget: {msg}"
                );
            }
            other => panic!("expected Failed after the budget, got {other:?}"),
        }
    }

    // --- Scene-settle gate (the 377 `waitSceneReady` + settle idea) ---

    /// A perform step whose send lands run energy 99 **and** drops the
    /// scene to 1 (a cheat tele's rebuild): the arm `stat(16) >= 99`
    /// holds once the rebuilt snapshot decodes the 99, so any progress
    /// during the hold must be the gate, not the arm.
    fn scene_drop_scenario(budget: u32) -> Scenario {
        Scenario {
            name: "scene-settle",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "tele and land run energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 99;
                        c.scene_state = 1; // the tele's scene rebuild
                        true
                    }),
                },
                wait: wait(Proof::Stat { id: 16, min: 99 }, budget),
            }],
            proof: Proof::Stat { id: 16, min: 99 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    /// With `set_scene_settle(Duration::ZERO)` the hold is only
    /// `scene_state == 2`: a perform send that drops the scene to 1 must
    /// not advance while the scene stays 1 (even though the arm holds on
    /// the rebuilt snapshot), and the arm fires the tick the scene
    /// returns to 2.
    #[test]
    fn scene_settle_zero_holds_while_scene_state_is_not_2() {
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(scene_drop_scenario(100));
        runner.set_scene_settle(Duration::ZERO);
        // Tick 1: seed completes; the send drops the scene to 1.
        runner.tick(&mut c);
        assert_eq!(
            c.scene_state, 1,
            "the send dropped the scene (tele rebuild)"
        );
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
        // Scene still 1: the snapshot now decodes run energy 99 (the arm
        // would hold), but the runner must hold — no step advance.
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "holds while scene_state != 2 even when the arm holds"
        );
        // Scene returns to 2: with a zero settle the gate releases and
        // the arm fires on the same tick.
        c.scene_state = 2;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.outcome, "PASS");
        assert_eq!(ev.predicate, "stat(16)>=99");
    }

    /// The wall-clock settle: with a non-zero `scene_settle` the runner
    /// holds even while the scene is 2, until the scene has held 2 for
    /// the settle. The first step's send is held too, so a tele's rebuild
    /// cannot be stepped onto.
    #[test]
    fn scene_settle_holds_steps_until_the_scene_has_held_2_for_the_settle() {
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(scene_drop_scenario(100));
        runner.set_scene_settle(Duration::from_millis(30));
        // Tick 1: the scene just became 2 (the seed completed); the
        // settle has not elapsed, so the step's send is held back.
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
        assert_eq!(c.runenergy, 0, "the send is held until the settle elapses");
        // After the settle: the send fires and drops the scene to 1.
        std::thread::sleep(Duration::from_millis(40));
        runner.tick(&mut c);
        assert_eq!(c.runenergy, 99, "the send fired once the settle elapsed");
        assert_eq!(c.scene_state, 1);
        // Scene 1: hold, regardless of the elapsed settle.
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "a scene drop below 2 resets the settle clock"
        );
        // Scene back to 2: the arm holds on the snapshot, but the settle
        // has not elapsed again, so the step still does not advance.
        c.scene_state = 2;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "the step waits out the settle even after the scene returns to 2"
        );
        // Once the settle elapses, the arm fires and the run passes.
        std::thread::sleep(Duration::from_millis(40));
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
    }

    // --- Companion slots (fleet runs) ---

    /// A 2-profile scenario: profile 0 is the driven slot, profile 1 is a
    /// companion whose per-frame hook flips a captured flag. Ticking the
    /// driven slot runs the walker steps; ticking the companion slot runs
    /// the hook.
    #[test]
    fn companion_slots_tick_their_hook_while_the_driven_slot_runs() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let flipped = Arc::new(AtomicBool::new(false));
        let hook = Arc::clone(&flipped);
        let scenario = Scenario {
            name: "companion",
            seed: Seed {
                profiles: vec![("test", "test"), ("test2", "test2")],
                mainland: false,
            },
            steps: vec![Step {
                name: "set run energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 99;
                        true
                    }),
                },
                wait: wait(Proof::Stat { id: 16, min: 1 }, 10),
            }],
            proof: Proof::Stat { id: 16, min: 1 },
            companions: vec![Companion {
                profile: 1,
                per_frame: Box::new(move |_| {
                    hook.store(true, Ordering::Relaxed);
                }),
            }],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        assert_eq!(runner.companion_profile_name(0), "test2");
        assert_eq!(runner.companion_for("test2"), Some(0));
        assert_eq!(runner.companion_for("nobody"), None);

        // Driven slot: the walker's steps run to PASS.
        let mut c = seeded_client();
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);

        // Companion slot: the hook flips the flag on tick.
        assert!(!flipped.load(Ordering::Relaxed));
        let index = runner
            .companion_for("test2")
            .expect("the companion slot resolves by profile name");
        runner.companion_tick(index, &mut c);
        assert!(flipped.load(Ordering::Relaxed));
    }

    /// Live boots mint per-run usernames: the runner drives/companions the
    /// minted names (never the registry's `test`/`test2`), so the vault's
    /// fresh accounts are the slots its per-frame hooks tick.
    #[test]
    fn live_names_replace_the_seed_names_for_dispatch() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let flipped = Arc::new(AtomicBool::new(false));
        let hook = Arc::clone(&flipped);
        let scenario = Scenario {
            name: "companion",
            seed: Seed {
                profiles: vec![("test", "test"), ("test2", "test2")],
                mainland: false,
            },
            steps: vec![Step {
                name: "set run energy",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        c.runenergy = 99;
                        true
                    }),
                },
                wait: wait(Proof::Stat { id: 16, min: 1 }, 10),
            }],
            proof: Proof::Stat { id: 16, min: 1 },
            companions: vec![Companion {
                profile: 1,
                per_frame: Box::new(move |_| {
                    hook.store(true, Ordering::Relaxed);
                }),
            }],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(scenario);
        let live = vec!["live1_0".to_string(), "live1_1".to_string()];
        runner.set_live_names(&live);
        runner.set_scene_settle(Duration::ZERO);
        assert_eq!(runner.profile_name(), "live1_0");
        assert_eq!(runner.companion_profile_name(0), "live1_1");
        assert!(runner.drives("live1_0"));
        assert!(
            !runner.drives("test"),
            "a live runner must not drive `test`"
        );
        assert_eq!(runner.companion_for("live1_1"), Some(0));
        assert_eq!(runner.companion_for("test2"), None);
    }

    #[test]
    fn terminal_shot_fires_the_sink_on_pass_and_fail() {
        type SinkEvents = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
        let sink_events: SinkEvents = Arc::new(Mutex::new(Vec::new()));
        let pass_sink = Arc::clone(&sink_events);
        let mut pass = ScenarioRunner::new(stat_scenario(1, 10));
        pass.set_scene_settle(Duration::ZERO);
        pass.set_terminal_shot("t-pass");
        pass.set_shot_sink(Box::new(move |label, snap| {
            pass_sink
                .lock()
                .unwrap()
                .push((label.to_string(), snap.tile()));
        }));
        let mut c = seeded_client();
        pass.tick(&mut c);
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        pass.tick(&mut c);
        assert_eq!(pass.status(), RunnerStatus::Passed);
        assert_eq!(
            sink_events.lock().unwrap().as_slice(),
            &[("t-pass".to_string(), Some((3220, 3220, 0)))],
            "the terminal shot fires once on PASS"
        );

        let fail_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let fail_sink = Arc::clone(&fail_events);
        let mut fail = ScenarioRunner::new(stat_scenario(999, 1));
        fail.set_scene_settle(Duration::ZERO);
        fail.set_terminal_shot("t-fail");
        fail.set_shot_sink(Box::new(move |label, _| {
            fail_sink.lock().unwrap().push(label.to_string());
        }));
        let mut c = seeded_client();
        fail.tick(&mut c);
        assert!(matches!(fail.status(), RunnerStatus::Failed(_)));
        assert_eq!(
            fail_events.lock().unwrap().as_slice(),
            &["t-fail".to_string()],
            "the terminal shot fires once on FAIL"
        );
    }

    #[test]
    fn deadline_fails_a_stuck_run() {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 20));
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        let mut runner = ScenarioRunner::new(stat_scenario(999, 999));
        runner.set_scene_settle(Duration::ZERO);
        runner.set_deadline(Duration::from_millis(1));
        let mut saw_failed = false;
        for _ in 0..20 {
            c.bump_gens(ServerProt::UPDATE_RUNENERGY);
            runner.tick(&mut c);
            if matches!(runner.status(), RunnerStatus::Failed(_)) {
                saw_failed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(saw_failed, "the deadline must fail a stuck run");
        match runner.status() {
            RunnerStatus::Failed(msg) => assert!(msg.contains("deadline"), "msg: {msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A one-step shot scenario: no send, the arm holds on the first
    /// dirty tick (stat(16) >= 0 with the seeded client), and the proof
    /// mirrors the arm.
    fn shot_scenario(label: &'static str) -> Scenario {
        Scenario {
            name: "shot",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "shot the courtyard",
                kind: StepKind::Shot { label },
                wait: wait(Proof::Stat { id: 16, min: 0 }, 10),
            }],
            proof: Proof::Stat { id: 16, min: 0 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "274bot-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn shot_step_fires_the_sink_once_with_label_and_terminal_snapshot() {
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
        runner.set_scene_settle(Duration::ZERO);
        type Fired = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
        let fired: Fired = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&fired);
        runner.set_shot_sink(Box::new(move |label, snap| {
            sink.lock().unwrap().push((label.to_string(), snap.tile()));
        }));
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        let fired = fired.lock().unwrap();
        assert_eq!(fired.len(), 1, "the sink fires once per shot step");
        assert_eq!(fired[0].0, "arrive courtyard");
        assert_eq!(
            fired[0].1,
            Some((3220, 3220, 0)),
            "the sink sees the terminal snapshot's player tile"
        );
    }

    fn start_script_scenario() -> Scenario {
        Scenario {
            name: "start-script",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![
                Step {
                    name: "last seed wait",
                    kind: StepKind::Shot { label: "seed" },
                    wait: wait(Proof::Stat { id: 16, min: 0 }, 1),
                },
                Step {
                    name: "start the catalog card",
                    kind: StepKind::StartScript,
                    wait: wait(Proof::Stat { id: 16, min: 0 }, 1),
                },
            ],
            proof: Proof::Stat { id: 16, min: 0 },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    #[test]
    fn start_script_step_is_a_client_noop_and_the_arm_holds_immediately() {
        let mut c = seeded_client();
        let energy = c.runenergy;
        let mut runner = ScenarioRunner::new(start_script_scenario());
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        assert!(
            runner.on_start_script(),
            "after the last seed wait, the live pumps see StartScript"
        );
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        assert_eq!(
            c.runenergy, energy,
            "StartScript is a no-op on the client (no cheat, no packet)"
        );
        assert!(
            !runner.on_start_script(),
            "the step has advanced; pumps must not Start again"
        );
    }

    #[test]
    fn shot_step_with_the_default_sink_is_a_noop_and_passes() {
        // The headless twin leaves the default (no-op) sink: a Shot step
        // must still run to PASS without a window to capture.
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
    }

    #[test]
    fn shot_step_writes_png_and_json_through_a_test_sink() {
        let dir = temp_dir("shot");
        let mut c = seeded_client();
        let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
        runner.set_scene_settle(Duration::ZERO);
        let sink_dir = dir.clone();
        let now = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_616_000);
        runner.set_shot_sink(Box::new(move |label, snap| {
            let json = serde_json::to_string_pretty(snap).expect("terminal snapshot serializes");
            // A known 4x4 RGBA buffer: no wgpu surface needed here.
            let rgba: Vec<u8> = (0..64).map(|i| (i * 4) as u8).collect();
            crate::shot::write_shot_at(&sink_dir, label, &rgba, 4, 4, &json, now)
                .expect("test sink writes the shot");
        }));
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
        let png = dir.join("2026-08-25T00-00-00_arrive_courtyard.png");
        let json = dir.join("2026-08-25T00-00-00_arrive_courtyard.json");
        assert!(png.exists(), "one <label>.png written: {png:?}");
        assert!(json.exists(), "one <label>.json written: {json:?}");
        let header = std::fs::read(&png).unwrap();
        assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(v["tile"], serde_json::json!([3220, 3220, 0]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relog_presses_the_logout_button_while_ingame() {
        let mut c = seeded_client();
        let com = IfType {
            client_code: api::interact::CC_LOGOUT,
            ..Default::default()
        };
        c.set_iface(2458, com);
        let scenario = Scenario {
            name: "relog",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "relog",
                kind: StepKind::Relog,
                wait: wait(
                    Proof::QuestDone {
                        name: "The Grand Tree",
                    },
                    10,
                ),
            }],
            proof: Proof::QuestDone {
                name: "The Grand Tree",
            },
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);
        assert_eq!(
            c.out.data().first().copied(),
            Some(ClientProt::IF_BUTTON.id as u8),
            "clean logout is IF_BUTTON, not a socket drop"
        );
        assert!(
            matches!(runner.status(), RunnerStatus::Running { .. }),
            "still waiting to come back; we do not DC-wait"
        );
    }

    const AIR_RUINS: (i32, i32, i32) = (2988, 3294, 0);
    const AIR_RUNE_ID: i32 = 556;
    const RUNE_ESSENCE_ID: i32 = 1436;
    const RUNECRAFT_STAT: i32 = 20;
    const CRAFT_XP: i32 = 5;

    fn set_inv(c: &mut Client, stacks: &[(i32, i32)]) {
        let types: Vec<i32> = stacks.iter().map(|(id, _)| id + 1).collect();
        let numbers: Vec<i32> = stacks.iter().map(|(_, n)| *n).collect();
        match c.iface_id(|f| f.r#type == ComponentType::TYPE_INV) {
            Some(id) => {
                let inv = c.iface_mut(id).unwrap();
                inv.link_obj_type = Some(types);
                inv.link_obj_number = Some(numbers);
            }
            None => {
                let id = c.push_iface(IfType {
                    r#type: ComponentType::TYPE_INV,
                    ..Default::default()
                });
                c.set_iface_mut(
                    id,
                    IfTypeMut {
                        link_obj_type: Some(types),
                        link_obj_number: Some(numbers),
                        ..Default::default()
                    },
                );
            }
        }
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
    }

    fn ruins_client() -> Client {
        let mut c = seeded_client();
        c.map_build_base_x = AIR_RUINS.0;
        c.map_build_base_z = AIR_RUINS.1;
        c.local_player = Some(ClientPlayer::at(0, 0));
        c.stat_xp[RUNECRAFT_STAT as usize] = 0;
        c.stat_effective_level[RUNECRAFT_STAT as usize] = 1;
        c.stat_base_level[RUNECRAFT_STAT as usize] = 1;
        set_inv(&mut c, &[(RUNE_ESSENCE_ID, 27)]);
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::UPDATE_STAT);
        c
    }

    fn apply_simultaneous_craft(c: &mut Client) {
        set_inv(c, &[(AIR_RUNE_ID, 27)]);
        c.stat_xp[RUNECRAFT_STAT as usize] = CRAFT_XP;
        c.bump_gens(ServerProt::UPDATE_STAT);
    }

    fn rune_watch_scenario(arms: &[Proof]) -> Scenario {
        Scenario {
            name: "rune-observation-order",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: arms
                .iter()
                .enumerate()
                .map(|(i, arm)| Step {
                    name: match i {
                        0 => "watch arrival at the selected mysterious ruins after Start",
                        1 if matches!(arm, Proof::StatXpGain { .. }) => {
                            "watch Runecraft XP from the selected craft"
                        }
                        1 => "watch essence become the selected rune after altar entry",
                        2 if matches!(arm, Proof::StatXpGain { .. }) => {
                            "watch Runecraft XP from the selected craft"
                        }
                        _ => "watch essence become the selected rune after altar entry",
                    },
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: wait(*arm, 4),
                })
                .collect(),
            proof: Proof::ItemId {
                id: AIR_RUNE_ID,
                count: 1,
            },
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    fn rc_baseline(runner: &ScenarioRunner) -> Option<i32> {
        runner
            .xp_baselines
            .iter()
            .find(|(id, _)| *id == RUNECRAFT_STAT)
            .map(|(_, xp)| *xp)
    }

    fn tick_until_done(runner: &mut ScenarioRunner, c: &mut Client) {
        for _ in 0..8 {
            if matches!(
                runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                return;
            }
            c.bump_gens(ServerProt::UPDATE_STAT);
            runner.tick(c);
        }
    }

    #[test]
    fn runecraft_xp_watch_before_product_catches_simultaneous_craft() {
        let ruins = Proof::ArrivedNear {
            x: AIR_RUINS.0,
            z: AIR_RUINS.1,
            level: AIR_RUINS.2,
            radius: 4,
        };
        let crafted = Proof::ItemId {
            id: AIR_RUNE_ID,
            count: 1,
        };
        let xp = Proof::StatXpGain {
            id: RUNECRAFT_STAT,
            min: 1,
        };

        let mut late_client = ruins_client();
        let mut late = ScenarioRunner::with_world(rune_watch_scenario(&[ruins, crafted, xp]), None);
        late.set_scene_settle(Duration::ZERO);
        late.tick(&mut late_client);
        assert_eq!(
            late.status(),
            RunnerStatus::Running { step: 1, total: 3 },
            "ruins arrival must arm the crafted-item watch first in the late order"
        );
        assert_eq!(rc_baseline(&late), None);
        apply_simultaneous_craft(&mut late_client);
        late.tick(&mut late_client);
        assert_eq!(
            rc_baseline(&late),
            Some(CRAFT_XP),
            "item-then-XP captures the already-applied craft XP as its baseline"
        );
        tick_until_done(&mut late, &mut late_client);
        match late.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("stat_xp_gain(20)>=1"),
                    "late XP watch must name the missed arm: {msg}"
                );
                assert!(
                    msg.contains("not seen within"),
                    "late XP watch must miss the same-snapshot gain: {msg}"
                );
            }
            other => panic!("late item-then-XP order must fail, got {other:?}"),
        }

        let mut corrected_client = ruins_client();
        let mut corrected =
            ScenarioRunner::with_world(rune_watch_scenario(&[ruins, xp, crafted]), None);
        corrected.set_scene_settle(Duration::ZERO);
        corrected.tick(&mut corrected_client);
        assert_eq!(
            corrected.status(),
            RunnerStatus::Running { step: 1, total: 3 },
            "ruins arrival must arm the XP watch before product"
        );
        assert_eq!(rc_baseline(&corrected), Some(0));
        apply_simultaneous_craft(&mut corrected_client);
        corrected.tick(&mut corrected_client);
        assert_eq!(
            rc_baseline(&corrected),
            Some(0),
            "XP armed at ruins keeps the pre-craft baseline"
        );
        tick_until_done(&mut corrected, &mut corrected_client);
        assert_eq!(corrected.status(), RunnerStatus::Passed);

        let mut seed_client = ruins_client();
        let mut seed_only =
            ScenarioRunner::with_world(rune_watch_scenario(&[ruins, xp, crafted]), None);
        seed_only.set_scene_settle(Duration::ZERO);
        seed_only.tick(&mut seed_client);
        tick_until_done(&mut seed_only, &mut seed_client);
        match seed_only.status() {
            RunnerStatus::Failed(msg) => {
                assert!(
                    msg.contains("stat_xp_gain(20)>=1"),
                    "seed-only essence at ruins must not satisfy XP: {msg}"
                );
            }
            other => panic!("seed-only input must not pass, got {other:?}"),
        }
    }

    #[test]
    fn fresh_xp_watch_does_not_consume_a_prior_step_gain() {
        const SKILL: i32 = 17;
        let cumulative = Proof::StatXpGain { id: SKILL, min: 1 };
        let fresh = Proof::FreshStatXpGain { id: SKILL, min: 1 };
        let scenario = Scenario {
            name: "fresh-xp",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![
                Step {
                    name: "watch the earlier gain",
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: wait(cumulative, 4),
                },
                Step {
                    name: "watch a later fresh gain",
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: wait(fresh, 4),
                },
            ],
            proof: fresh,
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut c = seeded_client();
        c.stat_xp[SKILL as usize] = 100;
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick(&mut c);
        c.stat_xp[SKILL as usize] = 101;
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 1, total: 2 },
            "the first gain advances only to the fresh watch"
        );

        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick(&mut c);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 1, total: 2 },
            "unchanged XP cannot reuse the prior step gain"
        );

        c.ingame = false;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        runner.tick(&mut c);
        assert_eq!(
            runner.fresh_xp_baseline, None,
            "leaving the session clears the step-local baseline"
        );
        c.ingame = true;
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick(&mut c);
        assert_eq!(runner.fresh_xp_baseline, Some((SKILL, 101)));

        c.stat_xp[SKILL as usize] = 102;
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick(&mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed);
    }

    const TEST_LAMP_ID: i32 = 2528;
    const TEST_STRENGTH_STAT: i32 = 2;
    const TEST_PRAYER_STAT: i32 = 5;
    const TEST_LAMP_AWARD: &str = "Your wish has been granted!";

    fn lamp_episode_scenario(budget_ticks: u32) -> Scenario {
        let fresh_prayer = Proof::FreshStatXpGain {
            id: TEST_PRAYER_STAT,
            min: 1,
        };
        Scenario {
            name: "lamp-episode",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![
                Step {
                    name: "observe one authentic lamp redemption episode",
                    kind: StepKind::ObserveLampRedemption {
                        lamp_id: TEST_LAMP_ID,
                        reward_stat: TEST_STRENGTH_STAT,
                    },
                    wait: wait(Proof::NoActiveContinue, budget_ticks),
                },
                Step {
                    name: "watch fresh script work after release",
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: wait(fresh_prayer, 4),
                },
            ],
            proof: fresh_prayer,
            companions: vec![],
            settings: ScenarioSettings::default(),
        }
    }

    fn set_continue_text(c: &mut Client, open: bool, text: &str) {
        use client::config::if_type::ButtonType;

        const ROOT: usize = 6206;
        const CONTINUE: usize = 6210;
        c.set_iface(
            ROOT,
            IfType {
                id: ROOT as i32,
                layer_id: ROOT as i32,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![CONTINUE as i32]),
                ..Default::default()
            },
        );
        c.set_iface(
            CONTINUE,
            IfType {
                id: CONTINUE as i32,
                layer_id: ROOT as i32,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            CONTINUE,
            IfTypeMut {
                text: text.into(),
                button_type: ButtonType::BUTTON_CONTINUE,
                ..Default::default()
            },
        );
        c.chat_modal_id = if open { ROOT as i32 } else { -1 };
        c.bump_gens(if open {
            ServerProt::IF_OPENCHAT
        } else {
            ServerProt::IF_CLOSE
        });
    }

    fn set_continue(c: &mut Client, open: bool) {
        set_continue_text(c, open, "");
    }

    #[test]
    fn lamp_episode_latches_same_frame_reward_then_requires_drain_release_and_fresh_work() {
        let mut c = seeded_client();
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
        c.stat_xp[TEST_PRAYER_STAT as usize] = 200;
        set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
        let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(12), None);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick_with_hold(&mut c, false);
        assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 2 });

        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(&mut c, true);

        set_inv(&mut c, &[]);
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
        c.bump_gens(ServerProt::UPDATE_STAT);
        set_continue_text(&mut c, true, TEST_LAMP_AWARD);
        runner.tick_with_hold(&mut c, true);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 2 },
            "reward, consumption, and active dialogue do not release a held episode"
        );

        set_continue(&mut c, false);
        runner.tick_with_hold(&mut c, true);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 2 },
            "dialogue drain alone does not release a still-held episode"
        );

        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(&mut c, false);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 1, total: 2 },
            "native hold release advances to the post-event work gate"
        );

        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick_with_hold(&mut c, false);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 1, total: 2 },
            "pre-release Prayer XP cannot satisfy the fresh post-release gate"
        );
        c.stat_xp[TEST_PRAYER_STAT as usize] = 201;
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick_with_hold(&mut c, false);
        assert_eq!(runner.status(), RunnerStatus::Passed);
    }

    #[test]
    fn lamp_episode_accepts_source_named_award_after_split_reward_packets() {
        let mut c = seeded_client();
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
        c.stat_xp[TEST_PRAYER_STAT as usize] = 200;
        set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
        let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(12), None);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick_with_hold(&mut c, true);

        // xplamp_confirm publishes the durable reward effects before mesbox;
        // a client frame may therefore observe these without the award IF.
        set_inv(&mut c, &[]);
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick_with_hold(&mut c, true);

        // The authentic award continuation arrives on a later frame, after
        // the reward/consumption edges have already been retained.
        set_continue_text(&mut c, true, TEST_LAMP_AWARD);
        runner.tick_with_hold(&mut c, true);
        set_continue_text(&mut c, false, "");
        runner.tick_with_hold(&mut c, true);
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(&mut c, false);

        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 1, total: 2 },
            "source-identified split award must complete the same held episode"
        );
    }

    #[test]
    fn lamp_episode_rejects_preexisting_and_later_unrelated_dialogues() {
        let mut c = seeded_client();
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
        set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
        set_continue_text(&mut c, true, TEST_LAMP_AWARD);
        let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(7), None);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick_with_hold(&mut c, true);
        set_inv(&mut c, &[]);
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
        c.bump_gens(ServerProt::UPDATE_STAT);
        runner.tick_with_hold(&mut c, true);
        set_continue(&mut c, false);
        runner.tick_with_hold(&mut c, true);
        set_continue(&mut c, true);
        runner.tick_with_hold(&mut c, true);
        set_continue(&mut c, false);
        runner.tick_with_hold(&mut c, false);

        for _ in 0..8 {
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick_with_hold(&mut c, false);
        }
        let RunnerStatus::Failed(message) = runner.status() else {
            panic!("preexisting and later unrelated dialogues must fail");
        };
        assert!(
            message.contains("lamp redemption episode"),
            "the failed native episode must be named: {message}"
        );
    }

    #[test]
    fn lamp_episode_rejects_wrong_hold_and_elapsed_only_progress() {
        let mut c = seeded_client();
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
        set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
        let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(5), None);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick_with_hold(&mut c, false);
        set_inv(&mut c, &[]);
        c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
        c.bump_gens(ServerProt::UPDATE_STAT);
        set_continue_text(&mut c, true, TEST_LAMP_AWARD);
        runner.tick_with_hold(&mut c, false);
        set_continue(&mut c, false);
        runner.tick_with_hold(&mut c, false);

        for _ in 0..6 {
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick_with_hold(&mut c, false);
        }
        let RunnerStatus::Failed(message) = runner.status() else {
            panic!("wrong native hold and elapsed ticks must fail");
        };
        assert!(message.contains("hold=false"), "{message}");
        assert!(
            message.contains("dialogue=false"),
            "an award outside a held episode must not bind: {message}"
        );
    }

    const TEST_MAZE_SPAWNS: &[WorldTile] = &[
        WorldTile {
            x: 2891,
            z: 4597,
            level: 0,
        },
        WorldTile {
            x: 2933,
            z: 4597,
            level: 0,
        },
        WorldTile {
            x: 2933,
            z: 4555,
            level: 0,
        },
        WorldTile {
            x: 2891,
            z: 4555,
            level: 0,
        },
    ];
    const TEST_MAZE_SHRINE: WorldTile = WorldTile {
        x: 2911,
        z: 4575,
        level: 0,
    };

    fn maze_episode_scenario(budget_ticks: u32) -> Scenario {
        Scenario {
            name: "maze-episode",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![Step {
                name: "observe ordered Maze completion",
                kind: StepKind::ObserveMazeCompletion {
                    spawns: TEST_MAZE_SPAWNS,
                    shrine: TEST_MAZE_SHRINE,
                    shrine_radius: 4,
                    min_progress: 8,
                    entry_shot: "maze entered",
                },
                wait: wait(Proof::IngameScene2, budget_ticks),
            }],
            proof: Proof::IngameScene2,
            companions: vec![],
            settings: ScenarioSettings {
                terminal_shot: Some("maze final"),
                ..ScenarioSettings::default()
            },
        }
    }

    fn set_world_tile(c: &mut Client, tile: WorldTile) {
        c.map_build_base_x = (tile.x >> 6) << 6;
        c.map_build_base_z = (tile.z >> 6) << 6;
        c.minusedlevel = tile.level;
        c.local_player = Some(ClientPlayer::at(
            tile.x - c.map_build_base_x,
            tile.z - c.map_build_base_z,
        ));
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        c.bump_gens(ServerProt::PLAYER_INFO);
    }

    fn tick_dirty(runner: &mut ScenarioRunner, c: &mut Client, hold: bool) {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(c, hold);
    }

    #[test]
    fn maze_episode_requires_held_entry_progress_shrine_return_reward_and_release() {
        let mut c = seeded_client();
        set_inv(&mut c, &[(995, 100)]);
        let mut runner = ScenarioRunner::with_world(maze_episode_scenario(20), None);
        runner.set_scene_settle(Duration::ZERO);
        type Captures = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
        let captures: Captures = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captures);
        runner.set_shot_sink(Box::new(move |label, snap| {
            sink.lock().unwrap().push((label.to_string(), snap.tile()));
        }));

        runner.tick_with_hold(&mut c, false);
        set_world_tile(&mut c, TEST_MAZE_SPAWNS[0]);
        runner.tick_with_hold(&mut c, true);
        assert_eq!(
            captures.lock().unwrap().as_slice(),
            &[("maze entered".to_string(), Some((2891, 4597, 0)))],
            "the entry capture reports the server-selected spawn"
        );

        set_world_tile(
            &mut c,
            WorldTile {
                x: 2891,
                z: 4590,
                level: 0,
            },
        );
        runner.tick_with_hold(&mut c, true);
        set_world_tile(
            &mut c,
            WorldTile {
                x: 2896,
                z: 4588,
                level: 0,
            },
        );
        runner.tick_with_hold(&mut c, true);
        set_world_tile(
            &mut c,
            WorldTile {
                x: 2910,
                z: 4576,
                level: 0,
            },
        );
        runner.tick_with_hold(&mut c, true);

        set_world_tile(
            &mut c,
            WorldTile {
                x: 3220,
                z: 3220,
                level: 0,
            },
        );
        set_inv(&mut c, &[(995, 101)]);
        runner.tick_with_hold(&mut c, true);
        assert_eq!(
            runner.status(),
            RunnerStatus::Running { step: 0, total: 1 },
            "server return and reward cannot pass while native hold remains"
        );
        tick_dirty(&mut runner, &mut c, false);

        assert_eq!(runner.status(), RunnerStatus::Passed);
        assert_eq!(
            captures.lock().unwrap().as_slice(),
            &[
                ("maze entered".to_string(), Some((2891, 4597, 0))),
                ("maze final".to_string(), Some((3220, 3220, 0))),
            ]
        );
        let evidence = runner.evidence().expect("terminal evidence");
        assert_eq!(evidence.scene, 2);
        assert_eq!(evidence.tile, Some([3220, 3220, 0]));
    }

    #[test]
    fn maze_episode_rejects_near_shrine_without_canonical_entry() {
        let mut c = seeded_client();
        set_inv(&mut c, &[(995, 100)]);
        let mut runner = ScenarioRunner::with_world(maze_episode_scenario(4), None);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick_with_hold(&mut c, false);

        set_world_tile(&mut c, TEST_MAZE_SHRINE);
        runner.tick_with_hold(&mut c, true);
        set_world_tile(
            &mut c,
            WorldTile {
                x: 3220,
                z: 3220,
                level: 0,
            },
        );
        set_inv(&mut c, &[(995, 101)]);
        for _ in 0..5 {
            tick_dirty(&mut runner, &mut c, false);
        }

        let RunnerStatus::Failed(message) = runner.status() else {
            panic!("near-shrine and outside planted states must fail");
        };
        assert!(message.contains("entry=None"), "{message}");
        assert!(message.contains("shrine=false"), "{message}");
    }

    #[test]
    fn maze_episode_rejects_return_without_real_progress_and_shrine_order() {
        let mut c = seeded_client();
        set_inv(&mut c, &[(995, 100)]);
        let mut runner = ScenarioRunner::with_world(maze_episode_scenario(5), None);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick_with_hold(&mut c, false);

        set_world_tile(&mut c, TEST_MAZE_SPAWNS[3]);
        runner.tick_with_hold(&mut c, true);
        set_world_tile(
            &mut c,
            WorldTile {
                x: 3220,
                z: 3220,
                level: 0,
            },
        );
        set_inv(&mut c, &[(995, 101)]);
        for _ in 0..6 {
            tick_dirty(&mut runner, &mut c, false);
        }

        let RunnerStatus::Failed(message) = runner.status() else {
            panic!("entry followed by a planted outside state must fail");
        };
        assert!(message.contains("progress=false"), "{message}");
        assert!(message.contains("shrine=false"), "{message}");
        assert!(message.contains("returned=false"), "{message}");
    }
}
