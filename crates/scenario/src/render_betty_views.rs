//! Headed fixed-orbit render captures: Betty → Falador corridor, dwarven wall,
//! and fountain-station water acceptance views. Each registered name is one
//! station × fixed yaw/pitch; the headed panel fires a single
//! [`ScenarioSettings::terminal_shot`] once the view gate holds (capture
//! completion, not visual qualification).

use std::time::Duration;

use api::interact::{cheat, tele_args};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;

use crate::{Proof, Scenario, ScenarioNav, ScenarioSettings, Seed, Step, StepKind, Wait};

/// Fixed orbit pitch for every matrix cell (client range 128–383).
pub const RENDER_BETTY_PITCH: i32 = 256;

/// Route matrix station S0 — Betty shop stand (shopPresets pin).
pub const STATION_BETTY: WorldTile = WorldTile {
    x: 3012,
    z: 3258,
    level: 0,
};

/// Route matrix station S6 — Falador street tile between Betty and the west bank leg.
pub const STATION_FALADOR_STREET: WorldTile = WorldTile {
    x: 3009,
    z: 3345,
    level: 0,
};

/// Route matrix station S8 — Falador west bank approach (level 0).
pub const STATION_WEST_BANK: WorldTile = WorldTile {
    x: 2945,
    z: 3368,
    level: 0,
};

/// Fountain tile used for water-render acceptance (manual headed reference view).
pub const STATION_FOUNTAIN: WorldTile = WorldTile {
    x: 3220,
    z: 3224,
    level: 0,
};

const RENDER_CAPTURE_DEADLINE: Duration = Duration::from_secs(300);
const RENDER_CAPTURE_STEP_BUDGET: u32 = 600;

/// Test-only-compatible view over the authoritative scenario registry.
#[cfg(test)]
#[derive(Clone, Copy)]
pub struct RenderNames;

#[cfg(test)]
pub const NAMES: RenderNames = RenderNames;

#[cfg(test)]
fn is_render_view_scenario(name: &str) -> bool {
    name.starts_with("render_betty_views_") || name.starts_with("render_fountain_")
}

#[cfg(test)]
impl RenderNames {
    pub fn len(&self) -> usize {
        crate::catalog::registered_names()
            .filter(|name| is_render_view_scenario(name))
            .count()
    }
}

#[cfg(test)]
impl IntoIterator for RenderNames {
    type Item = &'static &'static str;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        crate::catalog::registered_name_refs()
            .filter(|name| is_render_view_scenario(name))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

#[derive(Debug, Clone, Copy)]
struct ViewCase {
    scenario_name: &'static str,
    shot_label: &'static str,
    station: WorldTile,
    orbit_yaw: i32,
    orbit_pitch: i32,
}

const CASES: &[ViewCase] = &[
    ViewCase {
        scenario_name: "render_betty_views_betty_yaw0",
        shot_label: "betty_s0_yaw0",
        station: STATION_BETTY,
        orbit_yaw: 0,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    ViewCase {
        scenario_name: "render_betty_views_betty_yaw512",
        shot_label: "betty_s0_yaw512",
        station: STATION_BETTY,
        orbit_yaw: 512,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    ViewCase {
        scenario_name: "render_betty_views_falador_street_yaw0",
        shot_label: "falador_s6_yaw0",
        station: STATION_FALADOR_STREET,
        orbit_yaw: 0,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    ViewCase {
        scenario_name: "render_betty_views_falador_street_yaw512",
        shot_label: "falador_s6_yaw512",
        station: STATION_FALADOR_STREET,
        orbit_yaw: 512,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    ViewCase {
        scenario_name: "render_betty_views_west_bank_yaw0",
        shot_label: "west_bank_s8_yaw0",
        station: STATION_WEST_BANK,
        orbit_yaw: 0,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    ViewCase {
        scenario_name: "render_betty_views_west_bank_yaw512",
        shot_label: "west_bank_s8_yaw512",
        station: STATION_WEST_BANK,
        orbit_yaw: 512,
        orbit_pitch: RENDER_BETTY_PITCH,
    },
    // Operator's CPU cave-wall hole at this tile. Pitch128/yaw0 was recorded
    // on the same mine run; the operator frame itself has no atomic camera.
    ViewCase {
        scenario_name: "render_betty_views_dwarven_wall_yaw0",
        shot_label: "dwarven_wall_yaw0",
        station: WorldTile {
            x: 3011,
            z: 9813,
            level: 0,
        },
        orbit_yaw: 0,
        orbit_pitch: 128,
    },
    ViewCase {
        scenario_name: "render_fountain_yaw0_pitch128",
        shot_label: "fountain_yaw0_pitch128",
        station: STATION_FOUNTAIN,
        orbit_yaw: 0,
        orbit_pitch: 128,
    },
    ViewCase {
        scenario_name: "render_fountain_yaw0_pitch256",
        shot_label: "fountain_yaw0_pitch256",
        station: STATION_FOUNTAIN,
        orbit_yaw: 0,
        orbit_pitch: 256,
    },
    ViewCase {
        scenario_name: "render_fountain_yaw0_pitch383",
        shot_label: "fountain_yaw0_pitch383",
        station: STATION_FOUNTAIN,
        orbit_yaw: 0,
        orbit_pitch: 383,
    },
];

/// Lookup one matrix cell by its registered scenario name.
#[cfg(test)]
pub fn get(name: &str) -> Option<Scenario> {
    is_render_view_scenario(name)
        .then(|| crate::catalog::get(name))
        .flatten()
}

pub(crate) fn betty_yaw0_scenario() -> Scenario {
    scenario_for(CASES[0])
}

pub(crate) fn betty_yaw512_scenario() -> Scenario {
    scenario_for(CASES[1])
}

pub(crate) fn falador_street_yaw0_scenario() -> Scenario {
    scenario_for(CASES[2])
}

pub(crate) fn falador_street_yaw512_scenario() -> Scenario {
    scenario_for(CASES[3])
}

pub(crate) fn west_bank_yaw0_scenario() -> Scenario {
    scenario_for(CASES[4])
}

pub(crate) fn west_bank_yaw512_scenario() -> Scenario {
    scenario_for(CASES[5])
}

pub(crate) fn dwarven_wall_yaw0_scenario() -> Scenario {
    scenario_for(CASES[6])
}

pub(crate) fn fountain_yaw0_pitch128_scenario() -> Scenario {
    scenario_for(CASES[7])
}

pub(crate) fn fountain_yaw0_pitch256_scenario() -> Scenario {
    scenario_for(CASES[8])
}

pub(crate) fn fountain_yaw0_pitch383_scenario() -> Scenario {
    scenario_for(CASES[9])
}

fn scenario_for(case: ViewCase) -> Scenario {
    let gate = view_gate(case.station, case.orbit_yaw, case.orbit_pitch);
    let mut steps = crate::script_live_seed_steps();
    steps.push(Step {
        name: "diagnostic tele and fixed orbit camera",
        kind: StepKind::Perform {
            send: tele_and_orbit_send(case.station, case.orbit_yaw, case.orbit_pitch),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: case.station.x,
                z: case.station.z,
                level: case.station.level,
            },
            budget_ticks: RENDER_CAPTURE_STEP_BUDGET,
        },
    });
    steps.push(Step {
        // Drain continue/choice chat from late debug replies; tut_com_message
        // is cleared by the shared tutskip+relog seed above, not close_modal.
        name: "drain stray chat and settle render view",
        kind: StepKind::DrainDialogs { choice: 1 },
        wait: Wait {
            arm: gate,
            budget_ticks: 60,
        },
    });
    Scenario {
        name: case.scenario_name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: gate,
        companions: vec![],
        settings: ScenarioSettings {
            deadline: RENDER_CAPTURE_DEADLINE,
            terminal_shot: Some(case.shot_label),
            nav: ScenarioNav {
                camera_follow: false,
                show_nav_path: false,
                hop_labels: false,
                collision_fill: false,
                client_trail: false,
                nsew_labels: false,
                component_flood: false,
                ..ScenarioNav::default()
            },
            ..ScenarioSettings::default()
        },
    }
}

fn view_gate(station: WorldTile, orbit_yaw: i32, orbit_pitch: i32) -> Proof {
    Proof::RenderViewReady {
        x: station.x,
        z: station.z,
        level: station.level,
        orbit_yaw,
        orbit_pitch,
    }
}

type SnapshotPred = Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync>;

fn tele_and_orbit_send(station: WorldTile, orbit_yaw: i32, orbit_pitch: i32) -> SnapshotPred {
    Box::new(move |client, _| {
        cheat(client, &tele_args(station.level, station.x, station.z));
        apply_orbit_camera(client, orbit_yaw, orbit_pitch);
        true
    })
}

fn apply_orbit_camera(client: &mut Client, yaw: i32, pitch: i32) {
    client.orbit_camera_yaw = yaw & 0x7ff;
    client.orbit_camera_yaw_velocity = 0;
    client.orbit_camera_pitch = pitch.clamp(128, 383);
    client.orbit_camera_pitch_velocity = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StepKind;

    #[test]
    fn render_betty_views_registers_one_shot_cases() {
        assert_eq!(NAMES.len(), 10);
        for name in NAMES {
            let scenario = get(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(scenario.name, *name);
            assert_eq!(scenario.seed.profiles, [("test", "test")]);
            assert!(scenario.seed.mainland);
            assert_eq!(
                scenario.steps.len(),
                4,
                "{name} needs tutskip+relog seed before tele and capture gate"
            );
            assert!(matches!(scenario.steps[1].kind, StepKind::Relog));
            assert!(matches!(
                scenario.steps[1].wait.arm,
                Proof::SideTabAvailable { index: 3 }
            ));
            assert!(
                matches!(scenario.steps[2].kind, StepKind::Perform { .. }),
                "{name} tele must follow the shared live seed"
            );
            assert!(
                matches!(scenario.steps[3].kind, StepKind::DrainDialogs { .. }),
                "{name} drains stray chat before RenderViewReady"
            );
            assert!(
                !scenario
                    .steps
                    .iter()
                    .any(|st| matches!(st.kind, StepKind::Shot { .. })),
                "{name} must not queue an extra Shot step"
            );
        }
    }

    #[test]
    fn render_betty_views_case_identity_is_stable() {
        let betty0 = get("render_betty_views_betty_yaw0").expect("betty yaw0");
        assert_eq!(
            betty0.settings.terminal_shot,
            Some("betty_s0_yaw0"),
            "terminal shot label drives PNG+JSON naming"
        );
        assert!(matches!(
            betty0.steps[3].wait.arm,
            Proof::RenderViewReady {
                x: 3012,
                z: 3258,
                level: 0,
                orbit_yaw: 0,
                orbit_pitch: 256,
            }
        ));
        assert_eq!(betty0.proof.name(), betty0.steps[3].wait.arm.name());

        let west512 = get("render_betty_views_west_bank_yaw512").expect("west bank yaw512");
        assert_eq!(west512.settings.terminal_shot, Some("west_bank_s8_yaw512"));
        assert!(matches!(
            west512.steps[3].wait.arm,
            Proof::RenderViewReady {
                x: 2945,
                z: 3368,
                level: 0,
                orbit_yaw: 512,
                orbit_pitch: 256,
            }
        ));
    }

    #[test]
    fn render_fountain_views_pin_station_yaw_and_pitch() {
        let cases = [
            (
                "render_fountain_yaw0_pitch128",
                "fountain_yaw0_pitch128",
                128,
            ),
            (
                "render_fountain_yaw0_pitch256",
                "fountain_yaw0_pitch256",
                256,
            ),
            (
                "render_fountain_yaw0_pitch383",
                "fountain_yaw0_pitch383",
                383,
            ),
        ];
        for (name, shot, pitch) in cases {
            let scenario = get(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(scenario.settings.terminal_shot, Some(shot));
            match scenario.steps[3].wait.arm {
                Proof::RenderViewReady {
                    x,
                    z,
                    level,
                    orbit_yaw,
                    orbit_pitch,
                } => {
                    assert_eq!(
                        (x, z, level, orbit_yaw, orbit_pitch),
                        (3220, 3224, 0, 0, pitch)
                    );
                }
                other => panic!("{name} expected RenderViewReady, got {other:?}"),
            }
            assert_eq!(scenario.proof.name(), scenario.steps[3].wait.arm.name());
        }
    }

    #[test]
    fn render_betty_views_nav_overlay_stays_off() {
        for name in NAMES {
            let scenario = get(name).expect(name);
            let nav = &scenario.settings.nav;
            assert!(
                !nav.camera_follow
                    && !nav.show_nav_path
                    && !nav.collision_fill
                    && !nav.hop_labels
                    && !nav.client_trail,
                "{name} must stay overlay-free"
            );
        }
    }
}
