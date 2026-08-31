//! The shared scenario runner: a state machine both runners pump from
//! their own per-frame hooks. No sleeps inside — each `tick` is one
//! observe of the client (the panel slot thread headed, the
//! `run_with_io` hook headless), so "one scenario, two runners" shares
//! the exact same seed/step/proof logic.

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::obj_names::ObjNames;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use nav::router::{FindOptions, Route};
use nav::traveller::{TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;

use crate::evidence::Evidence;
use crate::proof::Proof;
use crate::{Scenario, StepKind, Wait};

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

/// The machine both runners drive. One instance per scenario run.
pub struct ScenarioRunner {
    scenario: Scenario,
    snapshot: GameSnapshot,
    obj_names: Option<Arc<ObjNames>>,
    /// The whole-world router surface (collision + transport graph),
    /// loaded from the baked v2 pack; both `Walk` and `Follow` steps
    /// route on it.
    nav_world: Option<Arc<NavWorld>>,
    traveller: Traveller,
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
    /// `None` while it is not 2. Any drop below 2 resets it.
    scene2_since: Option<Instant>,
    /// The seed waits for a mainland build base (>= 3000); tests on
    /// fixture grids relax this.
    require_mainland_base: bool,
    evidence: Option<Evidence>,
    /// Whole-window shot sink: fired once when a `StepKind::Shot` step's
    /// arm holds, with the label and the terminal snapshot. The headed
    /// panel fills this with its window capture; the headless twin keeps
    /// the default no-op.
    shot_sink: Option<Box<dyn FnMut(&str, &GameSnapshot) + Send>>,
}

impl ScenarioRunner {
    /// A runner for `scenario`; the whole-world router surface (collision
    /// + transport graph) loads from the standard pack path (`None` when
    /// no pack, so `Walk`/`Follow` steps fail with a clear message).
    pub fn new(scenario: Scenario) -> Self {
        let pack = crate::default_pack_path();
        let nav_world = NavWorld::load_pack(&pack).ok().map(Arc::new);
        Self::with_data(scenario, nav_world)
    }

    /// Runner with injected nav data: the whole-world [`NavWorld`]
    /// (collision + transport graph) both `Walk` and `Follow` steps route
    /// on. `None` keeps no world loaded, so a nav step fails with a clear
    /// "no nav world" message (the live [`ScenarioRunner::new`] loads the
    /// default pack path).
    pub fn with_world(scenario: Scenario, nav_world: Option<Arc<NavWorld>>) -> Self {
        Self::with_data(scenario, nav_world)
    }

    fn with_data(scenario: Scenario, nav_world: Option<Arc<NavWorld>>) -> Self {
        // Settings fields are Copy, so read them out before `scenario`
        // moves into the runner below.
        let deadline = scenario.settings.deadline;
        let terminal_shot = scenario.settings.terminal_shot;
        let require_mainland_base = scenario.settings.require_mainland_base;
        Self {
            scenario,
            snapshot: GameSnapshot::new(),
            obj_names: None,
            nav_world,
            traveller: Traveller::new(),
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
            require_mainland_base,
            evidence: None,
            shot_sink: None,
        }
    }

    /// Install the whole-window shot sink (headed: the panel's window
    /// capture bridge; headless: a no-op). Fired when a `Shot` step's
    /// arm holds.
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

    /// The profile the runner drives (`seed.profiles[0]`); per-frame hooks
    /// tick only this slot's client.
    pub fn profile_name(&self) -> &'static str {
        self.scenario.seed.profiles[0].0
    }

    /// Whether `name` is the slot this runner drives.
    pub fn drives(&self, name: &str) -> bool {
        name == self.profile_name()
    }

    /// The companion slot at `index`'s profile name
    /// (`seed.profiles[companion.profile].0`).
    pub fn companion_profile_name(&self, index: usize) -> &'static str {
        let profile = self.scenario.companions[index].profile;
        self.scenario.seed.profiles[profile].0
    }

    /// The companion slot whose profile name is `name`, `None` when no
    /// companion carries it. The driven slot (profile 0) is never a
    /// companion, so per-frame hooks dispatch: driven → [`ScenarioRunner::tick`],
    /// else `companion_for` → [`ScenarioRunner::companion_tick`].
    pub fn companion_for(&self, name: &str) -> Option<usize> {
        self.scenario
            .companions
            .iter()
            .position(|c| self.scenario.seed.profiles[c.profile].0 == name)
    }

    /// Run companion `index`'s per-frame hook once on `client`. No sleeps;
    /// the caller delivers one frame per call, exactly like [`tick`].
    pub fn companion_tick(&mut self, index: usize, client: &mut Client) {
        (self.scenario.companions[index].per_frame)(client);
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
        if matches!(self.phase, Phase::Done) {
            return;
        }
        let dirty = self.snapshot.rebuild(client);
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
            if !settled {
                return;
            }
            if dirty {
                self.step_route(client);
            }
            if !self.step_sent {
                if let Err(msg) = self.send_current(client) {
                    self.finish_fail(&format!(
                        "step {} ({}): {msg}",
                        self.step + 1,
                        self.current_step().name
                    ));
                    return;
                }
                self.step_sent = true;
                self.ticks_waited = 0;
            }
            if dirty {
                self.ticks_waited += 1;
                self.total_ticks += 1;
            }
            let (arm, budget) = {
                let wait = &self.current_step().wait;
                (wait.arm, wait.budget_ticks)
            };
            if arm.check(&self.snapshot, self.obj_names.as_deref()) {
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
            } else if self.ticks_waited >= budget {
                self.finish_fail(&format!(
                    "step {} ({}): {} not seen within {} ticks",
                    self.step + 1,
                    self.current_step().name,
                    arm.name(),
                    budget
                ));
            }
        }
        if matches!(self.phase, Phase::Proving)
            && self
                .scenario
                .proof
                .check(&self.snapshot, self.obj_names.as_deref())
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

    /// Seed complete: ingame, scene 2, a mainland build base, and — when a
    /// nav world is loaded — the player standing on a tile inside the
    /// packed collision's bounds. The base heuristic alone is not enough:
    /// the tutorial island's build base is also `>= 3000`, so the mainland
    /// hop must have actually landed (the pack covers Lumbridge only). A
    /// loc-blocked tele landing is still inside the bounds, so it releases
    /// the gate.
    fn seed_done(&self) -> bool {
        if !self.snapshot.ingame() || self.snapshot.scene_state() != 2 {
            return false;
        }
        if !self.require_mainland_base {
            return true;
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
        self.ticks_waited = 0;
        self.traveller.clear();
        self.route = None;
    }

    fn advance_step(&mut self) {
        self.step += 1;
        if self.step >= self.scenario.steps.len() {
            self.phase = Phase::Proving;
        } else {
            self.begin_step();
        }
    }

    /// Send the current step's action once. `Perform` runs its closure;
    /// `Walk` and `Follow` both arm the whole-world route (`find_with`
    /// over the collision + transport graph, default options) and drive
    /// `Traveller::follow`; `Shot` sends nothing (the step only waits for
    /// its arm to capture it).
    fn send_current(&mut self, client: &mut Client) -> Result<(), String> {
        match &self.scenario.steps[self.step].kind {
            StepKind::Perform { send } => {
                if send(client, &self.snapshot) {
                    return Ok(());
                }
                return Err("driver rejected the send".into());
            }
            StepKind::Shot { .. } => return Ok(()),
            StepKind::Walk { dest } | StepKind::Follow { dest } => {
                self.arm_route(*dest, FindOptions::default())
            }
        }
    }

    /// Arm a whole-world route: `nav::router::find` over the packed v2
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
        // cannot pay / has not earned is not relaxed.
        let state = nav::WorldState::from_snapshot(&self.snapshot);
        match nav::router::find_with(&world.collision, &world.graph, from, dest, opts, &state) {
            Ok(route) => {
                self.route = Some(route);
                Ok(())
            }
            Err(e) => Err(format!("no world path from {from:?} to {dest:?}: {e:?}")),
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
                "[nav-runner] tick={} snap={:?} phys={:?} dest={:?} legs={}",
                self.total_ticks,
                self.snapshot.tile(),
                phys,
                route.dest,
                route.legs.len(),
            );
        }
        // Exact dest: default close_enough=2 reports Arrived a tile or two
        // short, the proof (exact tile) fails, and the next poll re-starts
        // the original route — walker yo-yos back to the door.
        let mut options = TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        };
        let outcome = self
            .traveller
            .follow(client, &self.snapshot, route, &mut options);
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
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;
    use nav::grid::StepGrid;
    use nav::tile::Tile;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::{Companion, Scenario, ScenarioSettings, Seed, Step, StepKind};

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
    fn new_reads_deadline_and_terminal_shot_from_settings() {
        let runner = ScenarioRunner::new(crate::get("nav_full").unwrap());
        assert_eq!(runner.deadline(), Duration::from_secs(360));
        assert_eq!(runner.terminal_shot(), Some("nav_full terminal"));
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
        assert_eq!(ev.tile, Some([3220, 3220]));
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

        // A mainland build base releases the seed.
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 20));
        c.bump_gens(ServerProt::PLAYER_INFO);
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

        // Advance the player north one tile per tick; the follow polls its
        // settle on every delivered frame and the arm fires on the exact
        // destination tile.
        let mut steps = 0;
        while runner.status() != RunnerStatus::Passed {
            if steps > 80 {
                panic!("walk never arrived; status={:?}", runner.status());
            }
            steps += 1;
            c.local_player = Some(ClientPlayer::at(5, steps as i32));
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut c);
        }
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.tile, Some([3205, 3230]));
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
            c.local_player = Some(ClientPlayer::at(5, steps as i32));
            c.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut c);
        }
        let ev = runner.evidence().expect("evidence at PASS");
        assert_eq!(ev.tile, Some([3205, 3230]));
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

    #[test]
    fn terminal_shot_fires_the_sink_on_pass_and_fail() {
        let sink_events: Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>> =
            Arc::new(Mutex::new(Vec::new()));
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
        let fired: Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>> =
            Arc::new(Mutex::new(Vec::new()));
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
}
