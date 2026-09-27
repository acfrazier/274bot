use super::*;
pub(crate) const MINING_STAT: i32 = 14;
pub(crate) const RUNE_PICKAXE_ID: i32 = 1275;
pub(crate) const NOTED_COAL_ID: i32 = 454;
/// A Rune pickaxe plus 26 unstackable Knives leaves one slot for Coal.
/// CoalTrucks keeps non-coal items when it empties the pack into a truck.
pub(crate) const COAL_BALLAST_KNIVES: i32 = 26;
/// With the ordinary prayer/ranged/magic defaults, 48 in each melee/HP stat
/// yields native combat level 55 without over-leveling the fixture.
const COAL_MELEE_LEVEL: i32 = 48;

const COAL_MINE: WorldTile = WorldTile {
    x: 2582,
    z: 3481,
    level: 0,
};
pub(crate) const COAL_MINE_TRUCK_STAND: WorldTile = WorldTile {
    x: 2575,
    z: 3486,
    level: 0,
};
/// Seed Mining 60, ordinary combat-55 melee stats, Rune pickaxe 1275, and
/// 26 retained nonproduct Knives on the safe initial tile. The one free slot
/// makes the first mined Coal fill the pack. Observe real mining XP and exact
/// coal 453, then a mine-truck deposit (pack empty of coal at the truck stand,
/// not a Seers bank), then further mining.
/// Filling truck 120 then Seers haul/bank/return cannot fit
/// SCRIPT_GOLD_DEADLINE 180s from an empty truck; no truck-content seed
/// primitive exists. Death/combat recovery is not this core.
pub(crate) fn coal_trucks_scenario() -> Scenario {
    let stand = COAL_MINE;
    let first_xp = Proof::StatXpGain {
        id: MINING_STAT,
        min: 1,
    };
    let coal = Proof::ItemId {
        id: COAL_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed mining 60, combat-safe melee stats, Rune pickaxe and 26-Knife ballast",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat mining 60");
                cheat(c, &format!("setstat attack {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat strength {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat defence {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COAL_MELEE_LEVEL}"));
                cheat(c, "give rune_pickaxe 1");
                cheat(c, &format!("give knife {COAL_BALLAST_KNIVES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Attack 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Strength 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Defence 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 1,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 3,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Mining 60 before Start",
            Proof::Stat {
                id: MINING_STAT,
                min: 60,
            },
        ),
        (
            "confirm Rune pickaxe 1275 before Start",
            Proof::ItemId {
                id: RUNE_PICKAXE_ID,
                count: 1,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives and one available slot before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded coal in pack before Start",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted coal in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_COAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(Step {
        name: "teleport into the giant-bat mine after combat readiness is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Mining XP from coal rocks after Start", first_xp),
        ("watch exact Coal 453 mined after Start", coal),
        (
            "watch arrival at the mine coal truck after the pack fill",
            Proof::ArrivedNear {
                x: COAL_MINE_TRUCK_STAND.x,
                z: COAL_MINE_TRUCK_STAND.z,
                level: COAL_MINE_TRUCK_STAND.level,
                radius: 4,
            },
        ),
        (
            "watch the pack empty of coal after the mine-truck deposit",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        ("watch another exact Coal 453 after the truck deposit", coal),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "coal_trucks",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coal,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CoalTrucks"),
            terminal_shot: Some("coal_trucks"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
