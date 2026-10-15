use super::script_basics::script_live_seed_steps;
use crate::*;

/// Inside the selected-289 `woodcutting.draynor` site (x 3082..=3136,
/// z 3200..=3256), on open ground in view of real normal trees: content
/// `m48_50.jm2` places loc 1276 at (3107,3229), (3111,3226), (3106,3231)
/// and (3105,3228), all within the r6 proof below. The previous start
/// (3217,3231) was a Lumbridge tree's own footprint with no 1276 in r6,
/// so the pre-Start tree proof could never hold on 289.
const GATHERER_START: WorldTile = WorldTile {
    x: 3109,
    z: 3228,
    level: 0,
};
const BRONZE_AXE_ID: i32 = 1351;
const NORMAL_TREE_ID: i32 = 1276;
const WOODCUTTING_STAT_ID: i32 = 8;
pub(crate) const GATHERER_START_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "skill",
        value: ScriptInjectValue::Str("Woodcutting"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Start"),
    },
    ScriptSettingInject {
        id: "woodcuttingResources",
        value: ScriptInjectValue::StrList(&["normal"]),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];
const GATHERER_WATCH_TICKS: u32 = 240;

/// Starts the compiled Gatherer with a real axe at a live normal tree and
/// requires fresh Woodcutting XP after Start.
pub(crate) fn gatherer_scenario() -> Scenario {
    let fresh_xp = Proof::FreshStatXpGain {
        id: WOODCUTTING_STAT_ID,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Woodcutting 1 and a real bronze axe before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, "setstat woodcutting 1").is_sent()
                    && cheat(client, "give bronze_axe 1").is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: BRONZE_AXE_ID,
                count: 1,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "equip the observed bronze axe before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                matches!(
                    Interactions::new(snapshot, client).wear(BRONZE_AXE_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id: BRONZE_AXE_ID },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "stand at the live Draynor normal tree before Start",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, &tele_args(0, GATHERER_START.x, GATHERER_START.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: GATHERER_START.x,
                z: GATHERER_START.z,
                level: GATHERER_START.level,
            },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "confirm the selected-289 normal tree is live before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::LocIdNear {
                id: NORMAL_TREE_ID,
                x: GATHERER_START.x,
                z: GATHERER_START.z,
                level: GATHERER_START.level,
                radius: 6,
            },
            budget_ticks: 120,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch the compiled Gatherer produce fresh Woodcutting XP",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: fresh_xp,
            budget_ticks: GATHERER_WATCH_TICKS,
        },
    });

    Scenario {
        name: "gatherer",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::StatXpGain {
            id: WOODCUTTING_STAT_ID,
            min: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(240),
            start_script: Some("Gatherer"),
            script_settings_inject: Some(GATHERER_START_INJECT),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{EntityId, FamilyPreparation, Knowledge};
    use client::io::ClientRevision;
    use std::sync::Arc;

    /// The seed must stand inside the site it names, with a real placement
    /// of the proved tree id inside the proof radius — the two content
    /// facts whose absence made the old start unsatisfiable on 289.
    #[test]
    fn start_stands_in_draynor_with_a_proved_tree_in_reach() {
        let scenario = gatherer_scenario();
        let (id, x, z, level, radius) = scenario
            .steps
            .iter()
            .find_map(|step| match step.wait.arm {
                Proof::LocIdNear {
                    id,
                    x,
                    z,
                    level,
                    radius,
                } => Some((id, x, z, level, radius)),
                _ => None,
            })
            .expect("the scenario proves a live tree before Start");
        assert_eq!(
            (x, z, level),
            (GATHERER_START.x, GATHERER_START.z, GATHERER_START.level),
            "the tree proof is centred on the start tile"
        );

        let selected =
            api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data");
        let site = selected
            .gather_site("woodcutting", "woodcutting.draynor")
            .expect("the Draynor woodcutting site is published");
        let region = &site.region;
        assert!(
            region.min_x <= x
                && x <= region.max_x
                && region.min_z <= z
                && z <= region.max_z
                && region.level == level,
            "start ({x},{z},{level}) is outside woodcutting.draynor {region:?}"
        );

        let data = Arc::clone(&selected);
        let catalog = FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
            .expect("worker")
            .join()
            .expect("worker panicked")
            .expect("the gathering family prepares");
        let method = catalog
            .method("woodcutting.normal")
            .expect("the normal woodcutting method is published");
        let Knowledge::Known(spots) = &method.spots else {
            panic!(
                "normal woodcutting placements are not complete: {:?}",
                method.spots
            );
        };
        assert!(
            spots.iter().any(|spot| {
                spot.entity == EntityId::Loc(id)
                    && spot.origin.level == level
                    && (spot.origin.x - x).abs().max((spot.origin.z - z).abs()) <= radius
            }),
            "no loc {id} placement within r{radius} of the start ({x},{z},{level})"
        );
    }
}
