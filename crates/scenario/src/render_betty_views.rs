//! Headed render-route diagnostic captures along the Betty → Falador shop-buyout
//! corridor. Each registered name is one station × one fixed orbit yaw; the
//! headed panel fires a single [`ScenarioSettings::terminal_shot`] once the
//! view gate holds (capture completion, not visual qualification).

use std::time::Duration;

use api::interact::{cheat, tele_args};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;

use crate::{
    Proof, Scenario, ScenarioNav, ScenarioSettings, Seed, Step, StepKind, Wait,
};

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

const RENDER_CAPTURE_DEADLINE: Duration = Duration::from_secs(300);
const RENDER_CAPTURE_STEP_BUDGET: u32 = 600;

/// All six one-shot scenario names (one station × yaw per invocation).
pub const NAMES: &[&str] = &[
    "render_betty_views_betty_yaw0",
    "render_betty_views_betty_yaw512",
    "render_betty_views_falador_street_yaw0",
    "render_betty_views_falador_street_yaw512",
    "render_betty_views_west_bank_yaw0",
    "render_betty_views_west_bank_yaw512",
];

#[derive(Debug, Clone, Copy)]
struct ViewCase {
    scenario_name: &'static str,
    shot_label: &'static str,
    station: WorldTile,
    orbit_yaw: i32,
}

const CASES: &[ViewCase] = &[
    ViewCase {
        scenario_name: "render_betty_views_betty_yaw0",
        shot_label: "betty_s0_yaw0",
        station: STATION_BETTY,
        orbit_yaw: 0,
    },
    ViewCase {
        scenario_name: "render_betty_views_betty_yaw512",
        shot_label: "betty_s0_yaw512",
        station: STATION_BETTY,
        orbit_yaw: 512,
    },
    ViewCase {
        scenario_name: "render_betty_views_falador_street_yaw0",
        shot_label: "falador_s6_yaw0",
        station: STATION_FALADOR_STREET,
        orbit_yaw: 0,
    },
    ViewCase {
        scenario_name: "render_betty_views_falador_street_yaw512",
        shot_label: "falador_s6_yaw512",
        station: STATION_FALADOR_STREET,
        orbit_yaw: 512,
    },
    ViewCase {
        scenario_name: "render_betty_views_west_bank_yaw0",
        shot_label: "west_bank_s8_yaw0",
        station: STATION_WEST_BANK,
        orbit_yaw: 0,
    },
    ViewCase {
        scenario_name: "render_betty_views_west_bank_yaw512",
        shot_label: "west_bank_s8_yaw512",
        station: STATION_WEST_BANK,
        orbit_yaw: 512,
    },
];

/// Lookup one matrix cell by its registered scenario name.
pub fn get(name: &str) -> Option<Scenario> {
    CASES
        .iter()
        .find(|case| case.scenario_name == name)
        .map(|case| scenario_for(*case))
}

fn scenario_for(case: ViewCase) -> Scenario {
    let gate = view_gate(case.station, case.orbit_yaw, RENDER_BETTY_PITCH);
    let mut steps = crate::script_live_seed_steps();
    steps.push(Step {
        name: "diagnostic tele and fixed orbit camera",
        kind: StepKind::Perform {
            send: tele_and_orbit_send(case.station, case.orbit_yaw, RENDER_BETTY_PITCH),
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

fn tele_and_orbit_send(
    station: WorldTile,
    orbit_yaw: i32,
    orbit_pitch: i32,
) -> Box<dyn Fn(&mut Client, &GameSnapshot) -> bool + Send + Sync> {
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
    fn render_betty_views_registers_six_one_shot_cases() {
        assert_eq!(NAMES.len(), 6);
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
            assert!(matches!(
                scenario.steps[1].kind,
                StepKind::Relog
            ));
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
