use super::script_basics::script_live_seed_steps;
use crate::*;

/// Open ground in the selected-289 `woodcutting.draynor` site. `m48_50.jm2`
/// places normal trees at (3111,3217) and (3113,3217) (lines 5517 and 5578).
/// The closest jail guard spawns 24 tiles away (line 6688). The r12 search
/// and one-tile tree approach stay outside its five-tile hunt from r5 wander.
const GATHERER_START: WorldTile = WorldTile {
    x: 3112,
    z: 3213,
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
    steps.push(Step {
        name: "log out cleanly after the Gatherer XP proof",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::LoggedOut,
            budget_ticks: 600,
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
    use api::selected::{EntityId, Knowledge};
    use client::io::ClientRevision;
    use std::sync::Arc;

    #[test]
    fn start_search_area_stays_outside_jail_guard_hunt_range() {
        let radius = GATHERER_START_INJECT
            .iter()
            .find_map(|setting| match (setting.id, &setting.value) {
                ("radius", ScriptInjectValue::Num(radius)) => Some(*radius),
                _ => None,
            })
            .expect("Start mode sets a search radius");
        // m48_50.jm2:6688,6690-6691,6694 are the ground-level jailguard spawns.
        // draynor.npc:180 sets huntrange=5. NpcType.ts:101 defaults wanderrange to 5.
        // Include one tile for interaction beside a tree at the search edge.
        let guard_reach_from_spawn = 5.0 + 5.0;
        for (x, z) in [(3109, 3237), (3120, 3238), (3121, 3249), (3127, 3248)] {
            let distance = (x - GATHERER_START.x)
                .abs()
                .max((z - GATHERER_START.z).abs());
            assert!(
                f64::from(distance) > radius + 1.0 + guard_reach_from_spawn,
                "the Start area overlaps the hunt range of the jail guard at ({x},{z})"
            );
        }
    }

    /// The seed must stand inside the site it names, with a real placement
    /// of the proved tree id inside the proof radius — the two content
    /// facts whose absence made the old start unsatisfiable on 289.
    #[test]
    fn start_stands_in_draynor_with_a_proved_tree_in_reach() {
        if !api::test_support::game_data_available() {
            return;
        }
        let scenario = gatherer_scenario();
        let final_step = scenario
            .steps
            .last()
            .expect("the live Gatherer cell ends with a clean logout");
        assert!(matches!(&final_step.kind, StepKind::Relog));
        assert!(matches!(&final_step.wait.arm, Proof::LoggedOut));
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
        let input = script::preparation::PreparationInput::new(
            289,
            std::mem::size_of::<Arc<api::game_data::SelectedGameData>>(),
            2 * 1024 * 1024,
            2 * 1024 * 1024,
        )
        .unwrap()
        .with_gathering_family_decode();
        let catalog = script::preparation::submit(
            script::preparation::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            move |worker| data.prepare_gathering(worker),
        )
        .expect("admit the real gathering family")
        .join()
        .expect("worker settles")
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
