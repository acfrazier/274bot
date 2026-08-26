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
pub mod shot;
mod runner;

use std::path::PathBuf;
use std::time::Duration;

use api::snapshot::GameSnapshot;
use client::client::Client;
use nav::tile::Tile;

pub use evidence::{Evidence, InvRow, StatRow};
pub use proof::Proof;
pub use runner::{RunnerStatus, ScenarioRunner};

/// The default wall-clock deadline for a whole scenario run (seed + steps
/// + proof). The headless twin uses its own outer timeout.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(180);

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
        send: Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>,
    },
    /// Nav walk: arm the A* route from the current tile (the pack grid),
    /// hop it one leg per tick, wait for `arrived(dest)`.
    Walk { dest: Tile },
    /// Whole-window shot at the moment `wait.arm` holds: nothing is sent,
    /// then the runner fires the shot sink (headed: the panel captures
    /// the window; headless: a no-op) with the label + the terminal
    /// snapshot. The shot lands under `~/.274bot/smoke/<runId>/` as
    /// `<stamp>_<safeLabel>.png` + a `.json` sidecar.
    Shot { label: &'static str },
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
}

/// The registered scenario with this name, `None` when unknown.
pub fn get(name: &str) -> Option<Scenario> {
    match name {
        "walk" => Some(walk_scenario()),
        "render_smoke" => Some(render_smoke_scenario()),
        _ => None,
    }
}

/// Every registered scenario name (for the `--live script_<name>` usage).
pub fn names() -> Vec<&'static str> {
    vec!["walk", "render_smoke"]
}

/// The `render_smoke` scenario: log in `test`/`test`, do nothing, and
/// fire one whole-window shot the tick the seed gate releases. The panel's
/// `--smoke` path relaxes the mainland-base seed gate (`no_mainland_gate`)
/// so the capture lands the tick the focused slot first reaches
/// `ingame && scene_state == 2`; the `stat(16) >= 0` arm always holds on a
/// rebuilt snapshot, so the shot fires immediately. The proof mirrors the
/// arm so the run PASSes once the capture is requested — the panel exits 0
/// when the PNG is written, not on the runner status.
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
    }
}

/// The `walk` scenario: log in `test`/`test`, mainland-hop into the
/// Lumbridge courtyard, walk south across the open courtyard in two steps,
/// and prove the player is standing at (3220, 3212, 0). The landing tile
/// after the mainland tele is (3220, 3220) or (3220, 3222); both steps
/// walk through open, walkable tiles.
fn walk_scenario() -> Scenario {
    let mid = Tile {
        x: 3220,
        z: 3216,
        level: 0,
    };
    let dest = Tile {
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
    }
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
        Err(_) => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
            Err(_) => PathBuf::from(".274bot/274bot.navpack"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_smoke_registered_as_a_one_shot_scene2_capture() {
        let s = get("render_smoke").expect("render_smoke is registered");
        assert_eq!(s.name, "render_smoke");
        assert_eq!(s.seed.profiles, [("test", "test")]);
        assert!(!s.seed.mainland, "no hop needed for a smoke capture");
        assert_eq!(s.steps.len(), 1, "one capture step");
        assert!(matches!(s.steps[0].kind, StepKind::Shot { label: "scene2" }));
        assert_eq!(s.proof.name(), "stat(16)>=0");
    }
}
