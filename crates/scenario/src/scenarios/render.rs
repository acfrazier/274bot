use crate::*;

/// The `render_smoke` scenario: log in `test`/`test`, do nothing, and
/// fire one whole-window shot the tick the seed gate releases. The
/// scenario's own settings carry the relaxed mainland-base seed gate
/// (`require_mainland_base = false`), so the capture lands the tick the
/// focused slot first reaches `ingame && scene_state == 2`; the
/// `stat(16) >= 0` arm always holds on a rebuilt snapshot, so the shot
/// fires immediately. The proof mirrors the arm so the run PASSes once
/// the capture is requested — the panel exits 0 when the PNG is written,
/// not on the runner status.
pub(crate) fn render_smoke_scenario() -> Scenario {
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
        companions: vec![],
        settings: ScenarioSettings {
            deadline: Duration::from_secs(300),
            ..Default::default()
        },
    }
}
