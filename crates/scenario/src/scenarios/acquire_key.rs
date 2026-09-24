use super::pair::tele_step;
use super::script_basics::script_live_seed_steps;
use crate::*;

/// Matches `acquire_key_v2.ts` `STOP_OK`.
pub(crate) const ACQUIRE_KEY_V2_STOP: &str = "acquire key qualification complete";

/// Frozen `HERO_TILE.TAVERLEY_DUNGEON` (`heroquest/areas.ts:175`), the
/// ladder landing. It is about 108 tiles from the prison corridor
/// (2931,9690,0): no Jailer is in view, so the run must make its corridor
/// walk, and it is outside the jail cell.
pub(crate) const TAVERLEY_DUNGEON_LANDING: WorldTile = WorldTile {
    x: 2884,
    z: 9798,
    level: 0,
};
/// Melee levels that end a Jailer fight well inside the key family's 90s
/// kill bound.
const JAIL_COMBAT_LEVEL: i32 = 70;
const STRENGTH_STAT: i32 = 2;

/// A walk of about 108 tiles, a fight of up to 90s and, if the Jailer is
/// dead, his 75s respawn.
const ACQUIRE_KEY_DEADLINE: Duration = Duration::from_secs(360);
const ACQUIRE_KEY_WATCH: u32 = 560;

/// Pre-Start seed for the jail cards. A mainland seed cannot finish them:
/// the jail is in the Taverley dungeon, and the key card has to beat the
/// Jailer. So before Start the account gets melee levels, an empty pack
/// (plus the jail key for the cell card, whose own leg is the cell, not the
/// fight) and a teleport to the dungeon landing. Nothing changes the world
/// after Start.
pub(super) fn taverley_dungeon_seed_steps(jail_key: bool) -> Vec<Step> {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed melee levels and an empty pack for the jail before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                for stat in ["attack", "strength", "defence", "hitpoints"] {
                    cheat(c, &format!("setstat {stat} {JAIL_COMBAT_LEVEL}"));
                }
                cheat(c, "~clearinv");
                if jail_key {
                    cheat(c, "give jail_key 1");
                }
                true
            }),
        },
        wait: Wait {
            arm: if jail_key {
                Proof::Item {
                    name: "Jail key",
                    count: 1,
                }
            } else {
                Proof::Stat {
                    id: STRENGTH_STAT,
                    min: JAIL_COMBAT_LEVEL,
                }
            },
            budget_ticks: 120,
        },
    });
    steps.push(drain_setstat_levelups_before_hostile_tele());
    steps.push(tele_step(
        "teleport to the Taverley dungeon landing before Start",
        TAVERLEY_DUNGEON_LANDING,
    ));
    steps
}

/// Headed File witness: one Jailer leg from the dungeon landing. The gate
/// needs the host's corridor walk-near, arrival, Jailer attack, the jail key
/// in the pack and the settled `true` receipt.
pub(crate) fn acquire_key_v2_scenario() -> Scenario {
    let mut steps = taverley_dungeon_seed_steps(false);
    steps.push(start_catalog_step());
    steps.push(Step {
        name: "watch the File card reach its named helper stop",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            budget_ticks: ACQUIRE_KEY_WATCH,
            arm: Proof::Stat { id: 16, min: 0 },
        },
    });
    Scenario {
        name: "acquire_key_v2_ts",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ACQUIRE_KEY_DEADLINE,
            start_script: None,
            start_file: Some("acquire_key_v2.ts"),
            wait_script_stop: Some(ACQUIRE_KEY_V2_STOP),
            terminal_shot: Some("acquire_key_v2_ts"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Start comes after the account stands at `landing`, and nothing after
    /// Start changes the world.
    pub(in crate::scenarios) fn starts_at_the_landing_after_its_seed(
        s: &Scenario,
        landing: WorldTile,
    ) {
        let start = s
            .steps
            .iter()
            .position(|st| st.name == "start the catalog card")
            .expect("catalog Start");
        let tele = s
            .steps
            .iter()
            .position(|st| {
                matches!(
                    st.wait.arm,
                    Proof::Arrived { x, z, level } if (x, z, level) == (landing.x, landing.z, landing.level)
                )
            })
            .expect("a pre-Start teleport to the landing");
        assert!(tele < start, "the teleport lands before Start");
        assert!(
            !s.steps[start..].iter().any(|st| st.name.contains("setstat")
                || st.name.contains("setvar")
                || st.name.contains("seed")
                || st.name.contains("teleport")
                || st.name.contains("collision")
                || st.name.contains("npc")
                || st.name.contains("flag")),
            "no post-Start world mutation: {:?}",
            s.steps[start..]
                .iter()
                .map(|st| st.name)
                .collect::<Vec<_>>()
        );
        assert!(s.seed.mainland);
        assert!(s.settings.require_mainland_base);
    }

    #[test]
    fn key_card_starts_at_the_dungeon_landing_with_no_jail_key() {
        let s = acquire_key_v2_scenario();
        starts_at_the_landing_after_its_seed(&s, TAVERLEY_DUNGEON_LANDING);
        let seed = s
            .steps
            .iter()
            .find(|st| st.name.starts_with("seed melee levels"))
            .expect("the jail seed");
        assert!(
            !matches!(
                seed.wait.arm,
                Proof::Item {
                    name: "Jail key",
                    ..
                }
            ),
            "the key card must earn the jail key itself"
        );
        assert_eq!(s.settings.start_file, Some("acquire_key_v2.ts"));
        assert_eq!(s.settings.wait_script_stop, Some(ACQUIRE_KEY_V2_STOP));
    }
}
