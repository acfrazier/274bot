//! Per-slot readiness latch and outcome records for the memory harness.
//!
//! A fleet run fails closed when its slots do not all qualify. The aggregate
//! `ready/seeded/proved` counts say how many slots stopped, not which ones or
//! why, so the failure record and the observation-boundary records carry one
//! outcome per slot: whether it ever reached `ingame && scene_state == 2`,
//! and its last observed session, scenario and script state.

use crate::memory::Workload;
use crate::SlotStatus;
use scenario::RunnerStatus;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

/// Which slots have ever been `ingame && scene_state == 2`, and when. A slot
/// that flaps stays latched: "reached" is history, "now" is the status row.
pub(crate) struct ReadyLatch {
    index: HashMap<String, usize>,
    first: Vec<Option<Instant>>,
    ever: usize,
}

impl ReadyLatch {
    pub(crate) fn new(names: &[String]) -> Self {
        Self {
            index: names
                .iter()
                .enumerate()
                .map(|(i, name)| (name.clone(), i))
                .collect(),
            first: vec![None; names.len()],
            ever: 0,
        }
    }

    /// Latch each owned slot that is ready in `statuses`. Rows for slots this
    /// run does not own are ignored.
    pub(crate) fn observe(&mut self, statuses: &[SlotStatus], now: Instant) {
        for status in statuses {
            if !(status.ingame && status.scene_state == 2) {
                continue;
            }
            let Some(&slot) = self.index.get(status.username.as_str()) else {
                continue;
            };
            if self.first[slot].is_none() {
                self.first[slot] = Some(now);
                self.ever += 1;
            }
        }
    }

    /// Slots that have reached `ingame && scene_state == 2` at least once.
    pub(crate) fn ever(&self) -> usize {
        self.ever
    }

    pub(crate) fn first_ready(&self, name: &str) -> Option<Instant> {
        self.index.get(name).and_then(|&slot| self.first[slot])
    }
}

/// The seed runner state a slot record needs.
pub(crate) struct SeedView<'a> {
    pub status: RunnerStatus,
    /// The catalog script was started for this slot.
    pub started: bool,
    pub step_names: &'a [&'static str],
    /// The step the runner was last seen executing. A failed runner reports
    /// only its message, so this is what says where it stopped.
    pub last_step: Option<usize>,
}

impl SeedView<'_> {
    /// The runner proves the scenario's final predicate once every step has
    /// passed; that stage has no step name.
    fn step_name(&self, step: usize) -> &'static str {
        self.step_names.get(step).copied().unwrap_or("final proof")
    }

    fn to_json(&self) -> Value {
        match &self.status {
            RunnerStatus::Seeding => json!({"status": "seeding", "script_started": self.started}),
            RunnerStatus::Running { step, total } => json!({
                "status": "running",
                "step": step,
                "total": total,
                "step_name": self.step_name(*step),
                "script_started": self.started,
            }),
            RunnerStatus::Passed => json!({"status": "passed", "script_started": self.started}),
            RunnerStatus::Failed(message) => json!({
                "status": "failed",
                "message": message,
                "last_step": self.last_step,
                "last_step_name": self.last_step.map(|step| self.step_name(step)),
                "script_started": self.started,
            }),
        }
    }
}

/// The script slot state a record needs.
pub(crate) struct ScriptView {
    pub state: String,
    pub error: Option<String>,
    pub runtime: Value,
}

/// The last observed session state of a slot's status row.
fn client_json(status: &SlotStatus, now: Instant) -> Value {
    json!({
        "ingame": status.ingame,
        "scene_state": status.scene_state,
        "x": status.tile_x,
        "z": status.tile_z,
        "level": status.tile_level,
        "connected": status.connected,
        "startup_phase": format!("{:?}", status.startup_phase),
        "startup_phase_age_s": now
            .saturating_duration_since(status.startup_phase_started)
            .as_secs_f64(),
        "startup_progress": status.startup_progress_percent.map(|percent| json!({
            "percent": percent,
            "message": status.startup_progress_message,
        })),
        "error": status.error,
        "worker_terminal": status.worker_terminal.map(|terminal| format!("{terminal:?}")),
        "queue": (status.queue_position >= 0)
            .then(|| json!({"position": status.queue_position, "total": status.queue_total})),
        "login_latched": status.login_latched,
        "login_latch_reason": status.login_latch_reason.map(|reason| reason.to_string()),
        "welcome_hold": status.welcome_hold,
        "welcome_failure": status.welcome_failure,
    })
}

/// Whether a slot currently satisfies the run's own qualification definition:
/// ready alone for an unseeded idle fleet, seed complete with no running
/// script for a seeded idle one, and ready, proof passed and script running for
/// an active or lifecycle one. Readiness plus a non-failed seed is not enough.
pub(crate) fn slot_qualified(
    workload: Workload,
    has_card: bool,
    ready_now: bool,
    seed: Option<(&RunnerStatus, bool)>,
    script_running: bool,
) -> bool {
    let seeded =
        seed.is_some_and(|(status, on_start)| matches!(status, RunnerStatus::Passed) || on_start);
    let proved = seed.is_some_and(|(status, _)| matches!(status, RunnerStatus::Passed));
    ready_now
        && if workload == Workload::SeededIdle {
            seeded && !script_running
        } else if !has_card {
            true
        } else {
            seeded && proved && script_running
        }
}

/// One slot's outcome. The `state`, `error`, `runtime` and `client.{ingame,
/// scene_state,x,z,level}` keys are the observation-boundary record's
/// original contract; the rest says whether the slot ever qualified, whether
/// it satisfies the qualification definition now (`qualified_now`: the
/// authority for a stopped-at summary, since `reached_ingame_scene2` is
/// history and `state` alone says nothing about the proof) and where it stopped.
pub(crate) fn slot_outcome(
    name: &str,
    first_ready_s: Option<f64>,
    status: Option<&SlotStatus>,
    now: Instant,
    seed: Option<SeedView<'_>>,
    script: ScriptView,
    qualified_now: bool,
) -> Value {
    json!({
        "name": name,
        "reached_ingame_scene2": first_ready_s.is_some(),
        "first_ingame_scene2_s": first_ready_s,
        "ingame_scene2_now": status.is_some_and(|s| s.ingame && s.scene_state == 2),
        "qualified_now": qualified_now,
        "state": script.state,
        "error": script.error,
        "runtime": script.runtime,
        "client": status.map(|s| client_json(s, now)),
        "seed": seed.as_ref().map(SeedView::to_json),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StartupPhase;
    use std::time::Duration;

    fn row(name: &str, ingame: bool, scene_state: i32) -> SlotStatus {
        SlotStatus {
            username: name.into(),
            ingame,
            scene_state,
            ..SlotStatus::default()
        }
    }

    /// "Reached" is history: a slot that dropped out after qualifying stays
    /// counted, a slot that never got past scene loading does not, and rows
    /// for slots the run does not own never count.
    #[test]
    fn ready_history_survives_a_dropout_and_ignores_strangers() {
        let names = ["alice".to_string(), "bob".to_string(), "carol".to_string()];
        let mut latch = ReadyLatch::new(&names);
        let t0 = Instant::now();
        latch.observe(
            &[
                row("alice", true, 2),
                row("bob", true, 1),
                row("mallory", true, 2),
            ],
            t0,
        );
        assert_eq!(latch.ever(), 1);

        let t1 = t0 + Duration::from_secs(5);
        latch.observe(
            &[
                row("alice", false, 0),
                row("bob", true, 2),
                row("carol", false, 2),
            ],
            t1,
        );
        assert_eq!(latch.ever(), 2, "alice's dropout is not un-reached");
        assert_eq!(latch.first_ready("alice"), Some(t0));
        assert_eq!(latch.first_ready("bob"), Some(t1));
        assert_eq!(latch.first_ready("carol"), None, "ingame is required");
        assert_eq!(latch.first_ready("mallory"), None);

        latch.observe(&[row("alice", true, 2)], t1 + Duration::from_secs(9));
        assert_eq!(latch.first_ready("alice"), Some(t0), "first time is kept");
        assert_eq!(latch.ever(), 2);
    }

    fn script(state: &str, error: Option<&str>) -> ScriptView {
        ScriptView {
            state: state.into(),
            error: error.map(str::to_owned),
            runtime: json!({"dispatched": 3}),
        }
    }

    /// The record that answers "why did this slot not qualify": a slot stuck
    /// in the login queue says so and how long it has waited, while a slot
    /// that qualified but stalled mid-scenario names the step it is on.
    #[test]
    fn outcome_names_where_a_stuck_slot_stopped() {
        let now = Instant::now();
        let queued = SlotStatus {
            username: "alice".into(),
            startup_phase: StartupPhase::Queueing,
            startup_phase_started: now - Duration::from_secs(90),
            queue_position: 7,
            queue_total: 50,
            error: Some("world full".into()),
            ..SlotStatus::default()
        };
        let stuck = slot_outcome(
            "alice",
            None,
            Some(&queued),
            now,
            None,
            script("Idle", None),
            false,
        );
        assert_eq!(stuck["reached_ingame_scene2"], false);
        assert!(stuck["first_ingame_scene2_s"].is_null());
        assert_eq!(stuck["ingame_scene2_now"], false);
        assert_eq!(stuck["client"]["startup_phase"], "Queueing");
        assert_eq!(stuck["client"]["startup_phase_age_s"], 90.0);
        assert_eq!(stuck["client"]["queue"]["position"], 7);
        assert_eq!(stuck["client"]["error"], "world full");
        assert!(stuck["seed"].is_null());

        let names = ["prepare", "start", "watch restock"];
        let mid = slot_outcome(
            "bob",
            Some(12.5),
            Some(&row("bob", true, 2)),
            now,
            Some(SeedView {
                status: RunnerStatus::Running { step: 2, total: 3 },
                started: true,
                step_names: &names,
                last_step: Some(2),
            }),
            script("Running", Some("food option blank")),
            false,
        );
        assert_eq!(mid["reached_ingame_scene2"], true);
        assert_eq!(mid["first_ingame_scene2_s"], 12.5);
        assert_eq!(mid["ingame_scene2_now"], true);
        assert_eq!(mid["seed"]["step_name"], "watch restock");
        assert_eq!(mid["seed"]["script_started"], true);
        assert_eq!(mid["error"], "food option blank");
        assert!(
            mid["client"]["queue"].is_null(),
            "no queue place when not queued"
        );
    }

    /// After the last step the runner proves the scenario's final predicate;
    /// that stage has no step name and must not index past the list. A runner
    /// that failed reports only its message, so the record keeps the step it
    /// was last seen on: a slot that timed out mid-scenario names it, and one
    /// that never left the seed does not invent one.
    #[test]
    fn the_final_proof_stage_has_no_step_name_and_a_failure_names_its_last_step() {
        let names = ["only step"];
        let proving = SeedView {
            status: RunnerStatus::Running { step: 1, total: 1 },
            started: true,
            step_names: &names,
            last_step: Some(1),
        }
        .to_json();
        assert_eq!(proving["step_name"], "final proof");

        let failed_view = |last_step| SeedView {
            status: RunnerStatus::Failed("deadline".into()),
            started: true,
            step_names: &names,
            last_step,
        };
        let mid_scenario = failed_view(Some(0)).to_json();
        assert_eq!(mid_scenario["status"], "failed");
        assert_eq!(mid_scenario["message"], "deadline");
        assert_eq!(mid_scenario["last_step_name"], "only step");
        let in_proof = failed_view(Some(1)).to_json();
        assert_eq!(in_proof["last_step_name"], "final proof");
        let never_ran = failed_view(None).to_json();
        assert!(never_ran["last_step"].is_null());
        assert!(never_ran["last_step_name"].is_null());
    }

    fn qualified(
        workload: Workload,
        has_card: bool,
        ready: bool,
        seed: Option<(RunnerStatus, bool)>,
        running: bool,
    ) -> bool {
        slot_qualified(
            workload,
            has_card,
            ready,
            seed.as_ref().map(|(status, on_start)| (status, *on_start)),
            running,
        )
    }

    /// A slot is qualified only while it satisfies the run's own definition:
    /// an active slot needs a passed proof and a running script, so a ready,
    /// proven slot whose script has stopped is history, not qualification.
    #[test]
    fn an_active_slot_is_qualified_only_while_its_script_runs() {
        let passed = || Some((RunnerStatus::Passed, false));
        assert!(qualified(Workload::Active, true, true, passed(), true));
        assert!(!qualified(Workload::Active, true, true, passed(), false));
        assert!(!qualified(Workload::Active, true, false, passed(), true));
        let running = Some((RunnerStatus::Running { step: 1, total: 3 }, false));
        assert!(!qualified(Workload::Active, true, true, running, true));
        let waiting_for_script = Some((RunnerStatus::Seeding, true));
        assert!(
            !qualified(Workload::Active, true, true, waiting_for_script, true),
            "waiting at Start is seeded, not proved"
        );
        assert!(!qualified(Workload::Lifecycle, true, true, passed(), false));
        assert!(qualified(Workload::Lifecycle, true, true, passed(), true));
    }

    /// Idle fleets qualify on readiness alone; a seeded idle slot needs its
    /// seed complete (passed, or parked at Start) and its script not running.
    #[test]
    fn idle_slots_qualify_without_a_script() {
        assert!(qualified(Workload::Idle, false, true, None, false));
        assert!(!qualified(Workload::Idle, false, false, None, false));
        let parked = Some((RunnerStatus::Seeding, true));
        assert!(qualified(Workload::SeededIdle, false, true, parked, false));
        let unfinished = Some((RunnerStatus::Seeding, false));
        assert!(!qualified(
            Workload::SeededIdle,
            false,
            true,
            unfinished,
            false
        ));
        let passed = Some((RunnerStatus::Passed, false));
        assert!(!qualified(Workload::SeededIdle, false, true, passed, true));
    }
}
